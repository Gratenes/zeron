package sh.zeron.android.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import sh.zeron.android.R

/**
 * Debug builds: stand in for a Google Messages chat notification (a MessagingStyle one with a Reply action) and for
 * the app that gets the reply. `am broadcast -n sh.zeron.android/.phone.PhoneTestChatReceiver -a sh.zeron.android.TEST_CHAT
 * --es title "+15551234567" --es text "hello"` posts it; the reply is written to the action log and logcat.
 */
class PhoneTestChatReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val title = intent.getStringExtra("title") ?: "+15551234567"
        val text = intent.getStringExtra("text") ?: "hello"
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel("test-chat") == null) {
            manager.createNotificationChannel(NotificationChannel("test-chat", "Test chat", NotificationManager.IMPORTANCE_HIGH))
        }
        val input = RemoteInput.Builder("reply").setLabel("Reply").build()
        val reply = PendingIntent.getBroadcast(
            context, 1, Intent("sh.zeron.android.TEST_CHAT_REPLY").setPackage(context.packageName).putExtra("title", title),
            PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val action = Notification.Action.Builder(null, "Reply", reply).addRemoteInput(input).setSemanticAction(Notification.Action.SEMANTIC_ACTION_REPLY).build()
        val me = Person.Builder().setName("Me").build()
        val other = Person.Builder().setName(title).setUri("tel:$title").build()
        val style = NotificationCompat.MessagingStyle(me).setConversationTitle(null).addMessage(text, System.currentTimeMillis(), other)
        val n = Notification.Builder(context, "test-chat")
            .setSmallIcon(R.drawable.zi_arrow_up).setContentTitle(title).setContentText(text)
            .addAction(action)
        val built = NotificationCompat.Builder(context, "test-chat").setSmallIcon(R.drawable.zi_arrow_up).setStyle(style).build()
        built.actions = arrayOf(action)
        manager.notify(8801, built)
    }
}

class PhoneTestReplyReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString() ?: return
        android.util.Log.i("ZeronPhone", "TEST reply to ${intent.getStringExtra("title")}: $text")
        PhoneActionLog.add(context, "test_reply", "the test chat received a reply (${text.length} characters)", true, null)
    }
}
