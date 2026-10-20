package com.conduit.sync

import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NotificationMessageCompatTest {
    @Test fun olderApiMessageAdapterPreservesTextAndSender() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val me = Person.Builder().setName("Local user").build()
        val sender = Person.Builder().setName("Fixture sender").setKey("fixture-key").build()
        val notification = NotificationCompat.Builder(context, "ui-test")
            .setSmallIcon(R.drawable.ic_stat_link)
            .setStyle(NotificationCompat.MessagingStyle(me).addMessage("Fixture message", 42L, sender))
            .build()
        val messages = legacyMessagingMessages(notification, context)
        assertEquals(1, messages.size)
        assertEquals("Fixture message", messages.single().text.toString())
        assertEquals("Fixture sender", messages.single().senderPerson?.name.toString())
        assertEquals("fixture-key", messages.single().senderPerson?.key)
    }
}
