package com.conduit.sync

import android.app.Notification
import android.app.NotificationManager
import android.app.RemoteInput
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.NotifAction
import com.conduit.sync.proto.NotifActionDesc
import com.conduit.sync.proto.NotifNew
import com.conduit.sync.proto.NotifRemove
import com.conduit.sync.proto.NotifUpdate
import com.conduit.sync.proto.TextMessage
import com.google.protobuf.ByteString
import java.io.ByteArrayOutputStream

private const val TAG = "conduit.notif"

/** A toast shows two short lines; anything past this is padding nobody reads. */
internal const val NOTIF_MAX_TITLE = 200
internal const val NOTIF_MAX_TEXT = 2_000
internal const val NOTIF_MAX_MESSAGES = 3
internal const val NOTIF_MAX_MESSAGE_SENDER = 80
internal const val NOTIF_MAX_MESSAGE_TEXT = 320
private const val NOTIF_MAX_ACTIONS = 5
private const val NOTIF_MAX_ACTION_LABEL = 80
private const val NOTIF_MAX_REPLY = 2_000

/**
 * Keys we have already posted, so a repost becomes an update rather than a second toast.
 * Bounded on purpose: [onNotificationRemoved] normally prunes it, but a removal that
 * never arrives — the listener is rebound, the posting app is killed — must not leak.
 * 256 is far past any real shade.
 */
private const val REMEMBERED_KEYS = 256

/** Packages whose icon has already gone across. One per installed app that ever notifies. */
private const val REMEMBERED_PACKAGES = 64

/**
 * Windows draws `appLogoOverride` at 48 px on a standard-DPI toast, so 96 is already a
 * generous source and the bytes are the thing being economised.
 */
private const val ICON_PX = 96

/**
 * Per-icon byte ceiling, and the reason it is not a comfort setting: [WireSession.send]
 * *throws* on a payload past [MAX_PLAINTEXT], and [Link.send] turns any throw into a
 * teardown. Two of these plus the text fields have to leave that ceiling untroubled, or a
 * single fat avatar ends a session that is also carrying the clipboard. An icon over the
 * cap is dropped instead.
 */
internal const val ICON_MAX_BYTES = 24_000

/**
 * What a mirrored notification says when [Settings.hideNotificationContent] is on.
 *
 * Deliberately not Android's own "Sensitive notification content", which is the string the
 * platform substitutes when it decides a listener may not read a notification at all. Those
 * two look identical on a toast and have completely different fixes — one is a switch in this
 * app, the other is an appop — so they must not read the same.
 */
private const val HIDDEN_TITLE = "Notification hidden by Conduit"

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

    companion object {
        @Volatile private var live: NotificationRelay? = null

        /**
         * Called only after the user activates a mirrored Windows toast. Resolve the current
         * notification at that moment instead of retaining Android PendingIntents in another
         * cache. If the listener or notification disappeared meanwhile, the action is dropped.
         */
        fun perform(action: NotifAction) {
            val listener = live
            if (listener == null) {
                Log.w(TAG, "notification action dropped; listener is not connected")
                return
            }
            listener.performNow(action)
        }
    }

    /**
     * Access-ordered so eviction drops the least recently touched key, not the oldest. Keeping the
     * tiny bounded action descriptor list alongside the key lets us notice an action-list change
     * without a binder query or a second cache. Ordinary title/body reposts still use lightweight
     * NOTIF_UPDATE; only action changes rebuild the Windows toast XML.
     */
    private val posted = object : LinkedHashMap<String, List<NotifActionDesc>>(32, 0.75f, true) {
        override fun removeEldestEntry(eldest: Map.Entry<String, List<NotifActionDesc>>) =
            size > REMEMBERED_KEYS
    }

    /**
     * Packages whose app icon the desktop already has on disk, so it is sent once instead
     * of stapled to every notification — which for a day of chat is megabytes of the same
     * 8 kB PNG over a cellular relay.
     *
     * Cleared with [posted] when the listener is rebound, which doubles as the repair path:
     * if the desktop ever loses its icon cache, toggling notification access refills it.
     */
    private val iconSent = object : LinkedHashMap<String, Boolean>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: Map.Entry<String, Boolean>) =
            size > REMEMBERED_PACKAGES
    }

    override fun onListenerConnected() {
        // The system binds this on its own schedule and can do so before either the activity
        // or the service has run, so it loads the settings it reads rather than assuming
        // someone else already did. Idempotent.
        Settings.load(this)
        live = this
        // Logged because "notifications do not work" is almost always this never firing.
        Log.i(TAG, "listener connected")
    }

    override fun onListenerDisconnected() {
        Log.w(TAG, "listener disconnected")
        if (live === this) live = null
        posted.clear()
        iconSent.clear()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val link = SyncService.activeLink ?: return
        val ranking = ranking(sbn)
        if (!worthMirroring(sbn, ranking)) return

        val notification = sbn.notification
        val hide = Settings.hideNotificationContent
        val title = notification.extras.text(Notification.EXTRA_TITLE).take(NOTIF_MAX_TITLE)
        val body = notification.extras.text(Notification.EXTRA_TEXT)
            .ifEmpty { notification.extras.text(Notification.EXTRA_BIG_TEXT) }
            .take(NOTIF_MAX_TEXT)
        val messageRecords = if (hide) emptyList() else messagingMessages(notification)
        val messageDescs = textMessages(messageRecords)
        // Nothing to render. A media-session or progress-only notification lands here.
        if (title.isEmpty() && body.isEmpty() && messageDescs.isEmpty()) return

        // Redaction is applied here rather than by dropping the notification, so the desktop
        // still says *that* something arrived and from which app — the app name travels
        // separately and becomes the toast's source-app line. Applied after the emptiness
        // check above, so hiding cannot turn a notification worth nothing into one worth a
        // toast.
        val outTitle = if (hide) HIDDEN_TITLE else title
        val outBody = if (hide) "" else body
        val actionDescs = if (hide) emptyList() else actions(notification)

        // The same key posting again is an update — a chat thread gaining a message, a
        // download changing percentage — and must not pop a second toast. Actions are structural
        // toast XML rather than NotificationData, so a changed list deliberately falls through to
        // NOTIF_NEW with the same tag, replacing the existing toast with current buttons.
        val previousActions = posted.put(sbn.key, actionDescs)
        if (previousActions != null && previousActions == actionDescs) {
            val update = NotifUpdate.newBuilder()
                .setKey(sbn.key)
                .setTitle(outTitle)
                .setText(outBody)
                .addAllMessages(messageDescs)
                .build()
            link.send(Kind.NOTIF_UPDATE, update.toByteArray(), "notif update")
            return
        }

        val new = NotifNew.newBuilder()
            .setKey(sbn.key)
            .setPackage(sbn.packageName)
            .setAppName(appLabel(sbn.packageName))
            .setTag(sbn.tag.orEmpty())
            .setGroupKey(notification.group.orEmpty())
            .setTitle(outTitle)
            .setText(outBody)
            .setTimestampMs(sbn.postTime)
            .addAllMessages(messageDescs)
            .setSuppressPopup(previousActions != null)
        // A contact photo is content too, so hiding covers it. The app icon is not — it
        // says no more than the source-app line already does.
        if (!hide) face(notification, messageRecords)?.let { new.largeIconPng = ByteString.copyFrom(it) }
        new.addAllActions(actionDescs)
        // Marked sent even when the rasterise fails: the same package will fail the same
        // way, and retrying it on every notification buys nothing but the work.
        if (iconSent.put(sbn.packageName, true) == null) {
            appIcon(sbn.packageName)?.let { new.appIconPng = ByteString.copyFrom(it) }
        }
        Log.i(TAG, "notif out ${sbn.packageName} ${outTitle.take(40)} messages=${messageDescs.size}")
        link.send(Kind.NOTIF_NEW, new.build().toByteArray(), "notif")
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

    override fun onDestroy() {
        if (live === this) live = null
        super.onDestroy()
    }

    /**
     * Describes only what Windows can safely render: bounded labels, original Android action index,
     * and at most one free-form remote-input key per action. PendingIntents never leave Android.
     */
    private fun actions(notification: Notification): List<NotifActionDesc> =
        notification.actions.orEmpty()
            .mapIndexedNotNull { index, action ->
                val label = action.title?.toString()?.trim().orEmpty().take(NOTIF_MAX_ACTION_LABEL)
                if (label.isEmpty() || action.actionIntent == null) return@mapIndexedNotNull null
                val remote = action.remoteInputs?.firstOrNull { it.allowFreeFormInput }
                NotifActionDesc.newBuilder()
                    .setLabel(label)
                    .setIndex(index)
                    .setHasRemoteInput(remote != null)
                    .setResultKey(remote?.resultKey.orEmpty())
                    .build()
            }
            .take(NOTIF_MAX_ACTIONS)

    /**
     * Executes against the current StatusBarNotification. The echoed label/result key rejects a
     * stale Windows toast if the posting app reused the same key but changed its action list.
     * This binder query happens only on an actual user click, never while idle.
     */
    private fun performNow(request: NotifAction) {
        val sbn = runCatching { activeNotifications?.firstOrNull { it.key == request.key } }
            .onFailure { Log.w(TAG, "could not resolve notification action", it) }
            .getOrNull()
        if (sbn == null) {
            Log.i(TAG, "notification action dropped; ${request.key.take(48)} is no longer active")
            return
        }
        val action = sbn.notification.actions?.getOrNull(request.actionIndex)
        if (action == null) {
            Log.w(TAG, "notification action ${request.actionIndex} no longer exists")
            return
        }
        val currentLabel = action.title?.toString()?.trim().orEmpty().take(NOTIF_MAX_ACTION_LABEL)
        if (currentLabel != request.actionLabel) {
            Log.w(TAG, "notification action changed since it was mirrored; refusing stale click")
            return
        }

        val remote = action.remoteInputs?.firstOrNull { it.allowFreeFormInput }
        if (remote?.resultKey.orEmpty() != request.resultKey) {
            Log.w(TAG, "notification reply target changed since it was mirrored; refusing stale click")
            return
        }

        runCatching {
            if (remote != null) {
                val fillIn = Intent()
                val results = Bundle().apply {
                    putCharSequence(remote.resultKey, request.replyText.take(NOTIF_MAX_REPLY))
                }
                RemoteInput.addResultsToIntent(arrayOf(remote), fillIn, results)
                action.actionIntent.send(this, 0, fillIn)
            } else {
                action.actionIntent.send()
            }
        }.onSuccess {
            Log.i(TAG, "notification action executed: ${request.actionLabel.take(40)}")
        }.onFailure {
            Log.w(TAG, "notification action failed", it)
        }
    }

    /**
     * Ongoing notifications are the persistent kind — media transports, other apps'
     * foreground services, download progress bars. They are not events, so mirroring
     * them would leave permanent toasts. Group summaries duplicate their children.
     */
    private fun worthMirroring(
        sbn: StatusBarNotification,
        ranking: NotificationListenerService.Ranking?,
    ): Boolean {
        if (sbn.packageName == packageName) return false // our own ongoing link notice
        val notification = sbn.notification
        val flags = notification.flags
        if (flags and Notification.FLAG_ONGOING_EVENT != 0) return false
        if (flags and Notification.FLAG_GROUP_SUMMARY != 0) return false
        if (isMedia(notification)) return false
        if (isSilent(ranking)) return false
        return true
    }

    /**
     * A now-playing notification, which is a remote control rather than an event.
     *
     * The ongoing flag catches most of them, but not all: a scrobbler or a player that
     * refreshes its notification per track posts a fresh, non-ongoing one each time, which
     * would be a toast per song. The media session extra is the definitive marker — it is what
     * `MediaStyle` puts there and what the shade itself keys its player UI off.
     */
    private fun isMedia(notification: Notification): Boolean =
        notification.category == Notification.CATEGORY_TRANSPORT ||
            notification.extras.containsKey(Notification.EXTRA_MEDIA_SESSION)

    /**
     * Posted below [NotificationManager.IMPORTANCE_DEFAULT], which is Android's own definition
     * of silent: it makes no sound on the phone and sits quietly in the shade.
     *
     * Something the phone deliberately declined to interrupt for has not earned a toast on the
     * desktop either. Read off the ranking map the platform hands the listener, so it costs no
     * binder call — asking the channel its importance would be one per notification.
     *
     * A key missing from the map mirrors, rather than being dropped: not knowing is not a
     * reason to lose a message.
     */
    private fun ranking(sbn: StatusBarNotification): NotificationListenerService.Ranking? {
        val ranking = NotificationListenerService.Ranking()
        return ranking.takeIf { currentRanking.getRanking(sbn.key, it) }
    }

    private fun isSilent(ranking: NotificationListenerService.Ranking?): Boolean =
        ranking?.importance?.let { it < NotificationManager.IMPORTANCE_DEFAULT } ?: false

    /** Falls back to the package name, which is still better than an empty toast source. */
    private fun appLabel(pkg: String): String = runCatching {
        packageManager.getApplicationLabel(
            packageManager.getApplicationInfo(pkg, PackageManager.ApplicationInfoFlags.of(0)),
        ).toString()
    }.getOrDefault(pkg)

    /**
     * Reconstructs the public MessagingStyle records already embedded in the notification.
     * This is event-local work only: no active-notification query, shortcut lookup or provider read.
     */
    @Suppress("DEPRECATION")
    private fun messagingMessages(notification: Notification): List<Notification.MessagingStyle.Message> =
        runCatching {
            notification.extras
                .getParcelableArray(Notification.EXTRA_MESSAGES)
                ?.let(Notification.MessagingStyle.Message::getMessagesFromBundleArray)
                .orEmpty()
        }.getOrDefault(emptyList())

    /**
     * Sends only the newest few human-readable messages. Keeping this tiny is deliberate: the
     * notification also carries up to two PNGs in one Noise frame, and conversation history must
     * never be able to disconnect the clipboard by overflowing that frame.
     */
    private fun textMessages(records: List<Notification.MessagingStyle.Message>): List<TextMessage> =
        records.mapNotNull { message ->
            val text = message.text?.toString()?.trim().orEmpty().take(NOTIF_MAX_MESSAGE_TEXT)
            if (text.isEmpty()) return@mapNotNull null
            TextMessage.newBuilder()
                .setSender(
                    message.senderPerson?.name?.toString()?.trim().orEmpty()
                        .take(NOTIF_MAX_MESSAGE_SENDER),
                )
                .setText(text)
                .build()
        }.takeLast(NOTIF_MAX_MESSAGES)

    /**
     * The face for the toast, when the notification has one.
     *
     * The notification's large icon wins when present. Conversation-aware apps do not all fill it,
     * though: a genuine Nagram X notification on the target device had `largeIcon=null` while still
     * carrying `Notification.EXTRA_MESSAGES`. The platform's public
     * [Notification.MessagingStyle.Message.getMessagesFromBundleArray] reconstructs those message
     * records without AndroidX, and each message exposes its sender [android.app.Person] and icon.
     * The newest sender icon is therefore the fallback.
     *
     * This remains event-driven and in-memory. No shortcut query, provider read, timer, hidden API,
     * reflection, or background lookup is added.
     *
     * Caught, because this is another app's [Icon] pointing at another app's resources and
     * a notification must not be able to bring the listener down. A resource-type icon
     * remembers the package that created it, so `loadDrawable` resolves it against that
     * package and needs no context beyond ours.
     */
    @Suppress("DEPRECATION")
    private fun face(
        notification: Notification,
        messageRecords: List<Notification.MessagingStyle.Message>,
    ): ByteArray? = runCatching {
        val messageIcon = messageRecords.asReversed()
            .firstNotNullOfOrNull { message -> message.senderPerson?.icon }
        val icon = notification.getLargeIcon() ?: messageIcon
        icon?.loadDrawable(this)?.let { png(it) }
    }.getOrNull()

    private fun appIcon(pkg: String): ByteArray? =
        runCatching { png(packageManager.getApplicationIcon(pkg)) }.getOrNull()

    /**
     * Rasterises to a square PNG, or null if it came out over [ICON_MAX_BYTES].
     *
     * Runs on the main thread, like every callback here — a 96 px bitmap and its PNG
     * encode are a couple of milliseconds, and only on a notification that is new. The
     * alternative is a thread to do two `draw` calls on, which is the sort of thing this
     * project exists to not have.
     */
    private fun png(drawable: Drawable): ByteArray? {
        val bitmap = Bitmap.createBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888)
        // Aspect preserved and centred. A vector with no intrinsic size reports -1, which
        // coerces to a square and fills the canvas; a 4:3 contact photo stretched into a
        // square instead would give everyone a wide face, and Windows then crops a circle
        // out of the middle of that.
        val w = drawable.intrinsicWidth.coerceAtLeast(1)
        val h = drawable.intrinsicHeight.coerceAtLeast(1)
        val scale = ICON_PX.toFloat() / maxOf(w, h)
        val dw = (w * scale).toInt().coerceAtLeast(1)
        val dh = (h * scale).toInt().coerceAtLeast(1)
        drawable.setBounds(
            (ICON_PX - dw) / 2,
            (ICON_PX - dh) / 2,
            (ICON_PX + dw) / 2,
            (ICON_PX + dh) / 2,
        )
        drawable.draw(Canvas(bitmap))
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        bitmap.recycle()
        val bytes = out.toByteArray()
        if (bytes.size > ICON_MAX_BYTES) {
            Log.i(TAG, "icon is ${bytes.size} B, past the frame budget; sending none")
            return null
        }
        return bytes
    }
}

/** Extras are `CharSequence`, sometimes spanned, and very often absent. */
private fun android.os.Bundle.text(key: String): String =
    getCharSequence(key)?.toString()?.trim().orEmpty()
