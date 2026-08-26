package com.conduit.sync

import com.conduit.sync.proto.NotifNew
import com.conduit.sync.proto.TextMessage
import com.google.protobuf.ByteString
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The one thing about mirroring a notification that can break something else.
 *
 * [WireSession.send] refuses a payload past [MAX_PLAINTEXT] by throwing, and [Link.send]
 * turns any throw into a teardown — so a notification carrying two icons that together
 * overflow one frame would not merely fail to show, it would drop a session that is also
 * carrying the clipboard. The caps in [NotificationRelay] are chosen to make that
 * impossible, and this is the arithmetic actually being run rather than asserted in a
 * comment: raise any one of them past its share and this fails here instead of on a phone.
 */
class NotifBudgetTest {

    @Test
    fun the_fattest_possible_notification_still_fits_one_frame() {
        // Chinese, not emoji, and that is the point: `take()` counts UTF-16 chars, so the
        // worst byte-per-char ratio it can let through is a BMP character that costs 3
        // bytes in UTF-8. An emoji is 4 bytes but two chars, so it is cheaper per char.
        val worst = NotifNew.newBuilder()
            .setKey("0|com.example.a.very.long.messenger.package|1234567890|null|10123")
            .setPackage("com.example.a.very.long.messenger.package")
            .setAppName("应用名称".repeat(16))
            .setTag("t".repeat(128))
            .setGroupKey("g".repeat(256))
            .setTitle("标".repeat(NOTIF_MAX_TITLE))
            .setText("文".repeat(NOTIF_MAX_TEXT))
            .setTimestampMs(Long.MAX_VALUE)
            .addAllMessages(
                List(NOTIF_MAX_MESSAGES) {
                    TextMessage.newBuilder()
                        .setSender("发".repeat(NOTIF_MAX_MESSAGE_SENDER))
                        .setText("消".repeat(NOTIF_MAX_MESSAGE_TEXT))
                        .build()
                },
            )
            .setAppIconPng(ByteString.copyFrom(ByteArray(ICON_MAX_BYTES)))
            .setLargeIconPng(ByteString.copyFrom(ByteArray(ICON_MAX_BYTES)))
            .build()
            .toByteArray()

        // The envelope adds a kind and a length around this, so the payload has to clear
        // the ceiling with room rather than merely reach it.
        assertTrue(
            "a full notification is ${worst.size} B against a $MAX_PLAINTEXT B ceiling",
            worst.size < MAX_PLAINTEXT - 1_024,
        )
    }
}
