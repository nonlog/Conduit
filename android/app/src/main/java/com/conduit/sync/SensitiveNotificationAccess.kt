package com.conduit.sync

import android.app.AppOpsManager
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.os.Process
import android.util.Log

private const val TAG = "conduit.sensitive"
private const val SENSITIVE_NOTIFICATION_OP = "android:receive_sensitive_notifications"
private const val SENSITIVE_NOTIFICATION_OP_CLI = "RECEIVE_SENSITIVE_NOTIFICATIONS"

/**
 * Android 15+ can redact platform-marked sensitive notifications before they reach
 * NotificationListenerService. That trust decision is independent of Conduit's own hide-content
 * switch and its AppOp defaults to ignored for ordinary listeners.
 *
 * Checking is side-effect free. Repair is deliberately user-triggered: it invokes root once, sets
 * the AppOp, then toggles only Conduit's listener registration so Android refreshes its trusted
 * listener cache. No resident worker, polling, retry loop, or wakeup is added.
 */
internal object SensitiveNotificationAccess {
    fun isAllowed(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return true
        val appOps = context.getSystemService(AppOpsManager::class.java)
        return runCatching {
            appOps.unsafeCheckOpNoThrow(
                SENSITIVE_NOTIFICATION_OP,
                Process.myUid(),
                context.packageName,
            ) == AppOpsManager.MODE_ALLOWED
        }.onFailure {
            Log.w(TAG, "could not inspect sensitive-notification AppOp", it)
        }.getOrDefault(true)
    }

    fun repairWithRoot(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return true
        if (isAllowed(context)) return true

        val component = ComponentName(context, NotificationRelay::class.java).flattenToShortString()
        val command = buildString {
            append("cmd appops set ")
            append(context.packageName)
            append(' ')
            append(SENSITIVE_NOTIFICATION_OP_CLI)
            append(" allow && cmd notification disallow_listener ")
            append(component)
            append(" && cmd notification allow_listener ")
            append(component)
        }

        return runCatching {
            val process = ProcessBuilder("su", "-c", command)
                .redirectErrorStream(true)
                .start()
            val output = process.inputStream.bufferedReader().use { it.readText().trim() }
            val exitCode = process.waitFor()
            if (exitCode != 0) {
                Log.w(TAG, "root repair failed ($exitCode): ${output.take(240)}")
            } else {
                Log.i(TAG, "sensitive-notification AppOp repaired and listener rebound")
            }
            exitCode == 0 && isAllowed(context)
        }.onFailure {
            Log.w(TAG, "could not run rooted sensitive-notification repair", it)
        }.getOrDefault(false)
    }
}
