package sh.zeron.android.phone

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.telephony.PhoneNumberUtils
import androidx.core.app.NotificationCompat

/**
 * Google Messages has no API for answering an RCS chat, but every unread chat in the notification shade
 * carries its own Reply action (a RemoteInput). While Zeron has Notification access, this finds the reply
 * action of the notification for a conversation and fires it with the text: Messages then sends it in
 * the conversation's own transport. Nothing about the notifications is kept: only what is live is looked
 * at, when a reply is asked for.
 */
object PhoneChatReply {
    /** A conversation notification that can be answered. */
    class Handle(val key: String, val title: String, val pkg: String, val action: Notification.Action, val input: RemoteInput)

    const val MESSAGES = "com.google.android.apps.messaging"

    /** The apps whose chats may be answered: Google Messages (and, in debuggable builds only, a rig's choice). */
    fun allowedPackages(context: Context): Set<String> {
        val debuggable = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val extra = if (debuggable) context.getSharedPreferences("phone-access", Context.MODE_PRIVATE).getString("chat-packages", "").orEmpty()
            .split(',').map { it.trim() }.filter { it.isNotEmpty() } else emptyList()
        return setOf(MESSAGES) + extra
    }

    /** The live one-to-one chat notification for this person, or null. */
    fun find(context: Context, address: String, name: String?): Handle? {
        val listener = PhoneNotificationListener.instance ?: return null
        val allowed = allowedPackages(context)
        val notifications = runCatching { listener.activeNotifications }.getOrNull() ?: return null
        for (sbn in notifications) {
            if (sbn.packageName !in allowed) continue
            val handle = handleOf(sbn, address, name) ?: continue
            return handle
        }
        return null
    }

    private fun handleOf(sbn: StatusBarNotification, address: String, name: String?): Handle? {
        val n = sbn.notification
        if (n.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n) ?: return null
        if (style.isGroupConversation) return null
        val title = style.conversationTitle?.toString() ?: style.messages.firstOrNull()?.person?.name?.toString() ?: return null
        val digits = address.filter { it.isDigit() }
        val matches = (name != null && title.equals(name, ignoreCase = true)) ||
            (title.filter { it.isDigit() }.let { it.length >= 7 && PhoneNumberUtils.compare(title, address) }) ||
            style.messages.any { m ->
                val uri = m.person?.uri.orEmpty()
                uri.startsWith("tel:") && PhoneNumberUtils.compare(uri.removePrefix("tel:"), address) ||
                    (digits.length >= 7 && m.person?.key?.filter { it.isDigit() }?.endsWith(digits.takeLast(7)) == true)
            }
        if (!matches) return null
        val action = n.actions?.firstOrNull { a ->
            a.remoteInputs?.isNotEmpty() == true &&
                (a.semanticAction == Notification.Action.SEMANTIC_ACTION_REPLY || a.title.toString().contains("reply", ignoreCase = true))
        } ?: return null
        return Handle(sbn.key, title, sbn.packageName, action, action.remoteInputs.first())
    }

    /** Fires the reply action with [text]. False if the notification went away or refused. */
    fun send(context: Context, handle: Handle, text: String): Boolean {
        val fill = Intent()
        val results = Bundle().apply { putCharSequence(handle.input.resultKey, text) }
        RemoteInput.addResultsToIntent(arrayOf(handle.input), fill, results)
        if (Build.VERSION.SDK_INT >= 28) RemoteInput.setResultsSource(fill, RemoteInput.SOURCE_FREE_FORM_INPUT)
        return try {
            handle.action.actionIntent.send(context, 0, fill)
            true
        } catch (_: PendingIntent.CanceledException) {
            false
        }
    }
}

/**
 * Android binds this once the person turns on Notification access for Zeron (Settings → Notifications →
 * Notification access); it only lets [PhoneChatReply] look at the live notifications. It stores nothing.
 */
class PhoneNotificationListener : NotificationListenerService() {
    override fun onListenerConnected() {
        instance = this
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
    }

    companion object {
        @Volatile
        var instance: PhoneNotificationListener? = null
            private set
    }
}
