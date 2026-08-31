package com.conduit.sync

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean

private const val CLIP_ACTIVITY_TAG = "conduit.clip.focus"

/**
 * Translucent foreground handoff used only after an AccessibilityService copy event. Android 10+
 * grants clipboard reads to the focused app; once the exact ClipData is handed to the already-live
 * SyncService this activity immediately disappears. No history entry, no animation, no polling.
 */
class ClipboardChangeActivity : Activity() {
    companion object {
        private val running = AtomicBoolean(false)

        fun launch(context: Context) {
            if (!running.compareAndSet(false, true)) return
            runCatching {
                context.startActivity(
                    Intent(context, ClipboardChangeActivity::class.java).apply {
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                    },
                )
            }.onFailure {
                running.set(false)
                Log.w(CLIP_ACTIVITY_TAG, "could not request focused clipboard access", it)
            }
        }
    }

    private var handedOff = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setDimAmount(0f)
        @Suppress("DEPRECATION")
        overridePendingTransition(0, 0)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || handedOff) return
        handedOff = true
        if (LinkStatus.state == LinkState.Connected) {
            val clip = getSystemService(ClipboardManager::class.java).primaryClip
            if (clip != null && clip.itemCount > 0) {
                startService(
                    Intent(this, SyncService::class.java).apply {
                        action = ACTION_ACCESSIBILITY_CLIP
                        clipData = clip
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    },
                )
            }
        }
        finishWithoutAnimation()
    }

    override fun onDestroy() {
        running.set(false)
        super.onDestroy()
    }

    private fun finishWithoutAnimation() {
        finish()
        @Suppress("DEPRECATION")
        overridePendingTransition(0, 0)
    }
}
