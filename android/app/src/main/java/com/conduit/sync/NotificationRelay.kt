package com.conduit.sync

import android.app.Notification
import android.content.pm.PackageManager
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.NotifNew
import com.conduit.sync.proto.NotifRemove
import com.conduit.sync.proto.NotifUpdate

private const val TAG = "conduit.notif"

/** A toast shows two short lines; anything past this is padding nobody reads. */
private const val MAX_TITLE = 200
private const val MAX_TEXT = 2_000

/**
 * Keys we have already posted, so a repost becomes an update rather than a second toast.
 * Bounded on purpose: [onNotificationRemoved] normally prunes it, but a removal that
 * never arrives — the listener is rebound, the posting app is killed — must not leak.
 * 256 is far past any real shade.
 */
private const val REMEMBERED_KEYS = 256

/**
 * Mirrors the shade to the desktop.
 *
 * The system owns this service's lifecycle: it binds on boot and rebinds whenever it
 * feels like it, which is exactly why the relay cannot own a [Link]. It borrows
 * [SyncService]'s instead, and when nothing is connected the notification is simply
 * dropped — there is no queue here, because a notification the desktop missed is not
 * worth showing minutes later.
 *
 * Everything is edge-triggered by the platform callbacks. No polling, no timer, and no
 * call to `getActiveNotifications`, which would be a binder round trip per event.
 */
class NotificationRelay : NotificationListenerService() {

    /** Access-ordered so eviction drops the least recently touched key, not the oldest. */
    private val posted = object : LinkedHashMap<String, Boolean>(32, 0.75f, true) {
        override fun removeEldestEntry(eldest: Map.Entry<String, Boolean>) = size > REMEMBERED_KEYS
    }

    override fun onListenerConnected() {
        // Logged because "notifications do not work" is almost always this never firing.
        Log.i(TAG, "listener connected")
    }

    override fun onListenerDisconnected() {
        Log.w(TAG, "listener disconnected")
        posted.clear()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val link = SyncService.activeLink ?: return
        if (!worthMirroring(sbn)) return

        val notification = sbn.notification
        val title = notification.extras.text(Notification.EXTRA_TITLE).take(MAX_TITLE)
        val body = notification.extras.text(Notification.EXTRA_TEXT)
            .ifEmpty { notification.extras.text(Notification.EXTRA_BIG_TEXT) }
            .take(MAX_TEXT)
        // Nothing to render. A media-session or progress-only notification lands here.
        if (title.isEmpty() && body.isEmpty()) return

        // The same key posting again is an update — a chat thread gaining a message, a
        // download changing percentage — and must not pop a second toast.
        if (posted.put(sbn.key, true) != null) {
            val update = NotifUpdate.newBuilder()
                .setKey(sbn.key).setTitle(title).setText(body).build()
            link.send(Kind.NOTIF_UPDATE, update.toByteArray(), "notif update")
            return
        }

        val new = NotifNew.newBuilder()
            .setKey(sbn.key)
            .setPackage(sbn.packageName)
            .setAppName(appLabel(sbn.packageName))
            .setTag(sbn.tag.orEmpty())
            .setGroupKey(notification.group.orEmpty())
            .setTitle(title)
            .setText(body)
            .setTimestampMs(sbn.postTime)
            .build()
        Log.i(TAG, "notif out ${sbn.packageName} ${title.take(40)}")
        link.send(Kind.NOTIF_NEW, new.toByteArray(), "notif")
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        // Removed unconditionally: a key we never posted is absent anyway, and letting
        // the desktop hide a toast it does not have is harmless.
        posted.remove(sbn.key)
        val link = SyncService.activeLink ?: return
        val remove = NotifRemove.newBuilder()
            .setKey(sbn.key).setTag(sbn.tag.orEmpty()).setPackage(sbn.packageName).build()
        link.send(Kind.NOTIF_REMOVE, remove.toByteArray(), "notif remove")
    }

    /**
     * Ongoing notifications are the persistent kind — media transports, other apps'
     * foreground services, download progress bars. They are not events, so mirroring
     * them would leave permanent toasts. Group summaries duplicate their children.
     */
    private fun worthMirroring(sbn: StatusBarNotification): Boolean {
        if (sbn.packageName == packageName) return false // our own ongoing link notice
        val flags = sbn.notification.flags
        if (flags and Notification.FLAG_ONGOING_EVENT != 0) return false
        if (flags and Notification.FLAG_GROUP_SUMMARY != 0) return false
        return true
    }

    /** Falls back to the package name, which is still better than an empty toast source. */
    private fun appLabel(pkg: String): String = runCatching {
        packageManager.getApplicationLabel(
            packageManager.getApplicationInfo(pkg, PackageManager.ApplicationInfoFlags.of(0)),
        ).toString()
    }.getOrDefault(pkg)
}

/** Extras are `CharSequence`, sometimes spanned, and very often absent. */
private fun android.os.Bundle.text(key: String): String =
    getCharSequence(key)?.toString()?.trim().orEmpty()
