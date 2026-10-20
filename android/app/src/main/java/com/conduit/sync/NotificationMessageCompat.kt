package com.conduit.sync

import android.app.Notification
import android.app.Person
import android.content.Context
import androidx.core.app.NotificationCompat

/** API 29 uses the public AndroidX extractor; the platform bundle decoder starts at API 30. */
internal fun legacyMessagingMessages(notification: Notification, context: Context): List<Notification.MessagingStyle.Message> =
    NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)
        ?.messages.orEmpty().map { message ->
            val person = message.person?.let {
                Person.Builder().setName(it.name).setKey(it.key).setUri(it.uri)
                    .setBot(it.isBot).setImportant(it.isImportant)
                    .setIcon(it.icon?.toIcon(context)).build()
            }
            Notification.MessagingStyle.Message(message.text ?: "", message.timestamp, person).apply {
                if (message.dataMimeType != null && message.dataUri != null) {
                    setData(message.dataMimeType, message.dataUri)
                }
            }
        }
