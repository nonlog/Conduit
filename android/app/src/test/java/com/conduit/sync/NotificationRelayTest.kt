package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NotificationRelayTest {

    @Test
    fun same_key_and_content_with_new_post_time_is_dropped_but_content_change_is_sent() {
        val first = NotificationSnapshot(
            postTime = 1L,
            title = "新提醒📣",
            body = "🍗🍗🍗🍗🍗🍗 今天的签到收益是6个🍗，正常发挥",
            messages = emptyList(),
            actions = emptyList(),
        )
        val posted = mapOf("0|fork.risin42.nagramx|1326238159|null|10618" to first)

        val repost = first.copy(postTime = 2L)
        assertTrue(posted["0|fork.risin42.nagramx|1326238159|null|10618"] == repost)
        assertEquals(first.hashCode(), repost.hashCode())

        val changed = repost.copy(body = "🍗🍗🍗🍗🍗🍗 今天的签到收益是7个🍗，正常发挥")
        assertFalse(posted["0|fork.risin42.nagramx|1326238159|null|10618"] == changed)
    }
}
