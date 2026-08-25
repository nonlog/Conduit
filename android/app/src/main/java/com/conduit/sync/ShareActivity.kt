package com.conduit.sync

import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.widget.Toast

private const val TAG = "conduit.share"

/**
 * How many files one share may carry.
 *
 * ponytail: a flat cap, and the reason it exists is [Link]'s bounded sender queue — a
 * hundred-file share would push clips and notifications out of a 64-slot discard-oldest
 * queue and lose them silently. The excess is named in the log and in the toast rather than
 * dropped quietly. Raise it by giving transfers a queue of their own.
 */
private const val MAX_FILES = 16

/**
 * Conduit's entry in the system share sheet.
 *
 * No UI: it reads the intent, hands the work to [SyncService] and finishes, which is why the
 * theme is translucent — the share sheet dismisses and nothing of ours ever appears.
 *
 * The one subtle part is the URI permission. A share grants read access to *this* app for
 * the URIs on the intent, and that grant is tied to the component that received it. The
 * service is what actually opens the stream, possibly seconds later on its sender thread, so
 * the URIs are forwarded in a [ClipData] with [Intent.FLAG_GRANT_READ_URI_PERMISSION] — that
 * re-grants them to the service and ties the grant to its lifetime instead of to this
 * activity, which is gone by then.
 */
class ShareActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uris = uris(intent).distinct()
        val text = intent.getStringExtra(Intent.EXTRA_TEXT)
        val peer = Identity.peerName(filesDir) ?: "the desktop"

        // A share with no session would be queued onto a link that drops it, so the honest
        // answer is to say so and start a connection the next share can use. Sharing to
        // Conduit is also a clear statement that the user wants it linked, which is why this
        // overrides a remembered disconnect.
        if (LinkStatus.state != LinkState.Connected) {
            Log.w(TAG, "share arrived while ${LinkStatus.state}; connecting instead of sending")
            startForegroundService(
                Intent(this, SyncService::class.java).setAction(ACTION_CONNECT),
            )
            toast("Not linked to $peer yet — connecting, try again in a moment")
            finish()
            return
        }

        when {
            uris.isNotEmpty() -> {
                val sending = uris.take(MAX_FILES)
                if (uris.size > sending.size) {
                    Log.w(TAG, "share had ${uris.size} files, sending the first $MAX_FILES")
                }
                toast(
                    when {
                        !send(sending) -> "Can't read what was shared"
                        uris.size > sending.size ->
                            "Sending ${sending.size} of ${uris.size} files to $peer"
                        else -> "Sending ${sending.size} to $peer"
                    },
                )
            }
            // Sharing text is a share of a clip, which this app already knows how to do.
            // Handling it costs three lines and the alternative is a share-sheet entry that
            // silently does nothing when the user picks it for a link.
            !text.isNullOrBlank() -> {
                startForegroundService(
                    Intent(this, SyncService::class.java)
                        .setAction(ACTION_SHARE)
                        .putExtra(Intent.EXTRA_TEXT, text),
                )
                toast("Sent to $peer")
            }
            else -> {
                Log.w(TAG, "share intent ${intent.action} carried nothing we can send")
                toast("Nothing to send")
            }
        }
        finish()
    }

    /**
     * Hands the URIs to the service, or false if the grant cannot be passed on.
     *
     * A share is an intent from any app on the phone, so it is a trust boundary: an app may
     * name a URI it never actually granted us — or one its own provider refuses to let anyone
     * re-grant — and re-granting what we do not hold throws. Uncaught that would take this
     * process down, and [SyncService]'s live session with it, which is the one outcome a share
     * must never have.
     */
    private fun send(uris: List<Uri>): Boolean {
        // The first item builds the ClipData; the rest are added to it. The URIs have to
        // travel this way rather than as extras — a plain extra carries no grant.
        //
        // `newRawUri`, never `newUri`: the latter asks the provider for the URI's stream
        // types, which acquires the provider over binder — on the main thread, and it throws
        // SecurityException for any provider this app has no blanket access to (a document
        // picked out of Files, say). The MIME type it would have fetched is not even wanted
        // here: `Files.meta` asks for the real one on the sender thread.
        val clip = ClipData.newRawUri("conduit", uris.first())
        uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
        return runCatching {
            startForegroundService(
                // `setClipData` returns void, so this is an `apply` rather than a chain.
                Intent(this, SyncService::class.java).apply {
                    action = ACTION_SHARE
                    clipData = clip
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                },
            )
        }.onFailure { Log.w(TAG, "cannot pass ${uris.size} shared URIs to the service", it) }
            .isSuccess
    }

    /**
     * Every place a share can hide a URI.
     *
     * `EXTRA_STREAM` is the documented one, but plenty of apps put the URI only in the
     * intent's own [ClipData], and a share sheet entry that works for some apps and not
     * others is worse than one that does not exist.
     */
    @Suppress("DEPRECATION")
    private fun uris(intent: Intent): List<Uri> {
        val streams = when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))
            Intent.ACTION_SEND_MULTIPLE ->
                intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.filterNotNull()
                    ?: emptyList()
            else -> emptyList()
        }
        val clip = intent.clipData
        val fromClip = (0 until (clip?.itemCount ?: 0)).mapNotNull { clip?.getItemAt(it)?.uri }
        return streams + fromClip
    }

    private fun toast(message: String) =
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
}
