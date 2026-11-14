package sh.zeron.android.car

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.car.app.notification.CarPendingIntent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import sh.zeron.android.MainActivity
import sh.zeron.android.R

/**
 * A report as a messaging notification, for when the driver has left Zeron's
 * screen (for Maps, say): Android Auto shows it over whatever is showing and
 * Assistant reads it aloud and takes a spoken reply (see [CarReplyService]).
 * One per chat; opening the chat clears it.
 */
internal class CarNotifications(context: Context) {
    private val context = context.applicationContext
    private val manager = NotificationManagerCompat.from(this.context)

    init {
        this.context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, "Agent reports in the car", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "When an agent you talked to from Android Auto finishes or asks something"
            },
        )
    }

    /**
     * Post [report] as a conversation Android Auto reads aloud (Assistant),
     * with a reply the driver dictates and a mark-as-read. False when
     * notifications are off, so the caller can speak it itself.
     */
    fun post(report: Report): Boolean {
        val header = when (report.kind) {
            ReportKind.Finished -> "${report.title} is done"
            ReportKind.NeedsInput -> "${report.title} needs you"
            ReportKind.Failed -> "${report.title} failed"
        }
        return notify(report.chatId, report.title, header, CarMessaging.body(report), report.createdAtMs)
    }

    /** A dictated reply that didn't go through, said in the same conversation (so it can be retried). */
    fun failed(chatId: String, title: String, message: String) {
        notify(chatId, title, "Reply not sent", "Your reply wasn't sent. $message", System.currentTimeMillis())
    }

    private fun notify(chatId: String, title: String, header: String, body: String, atMs: Long): Boolean {
        if (!manager.areNotificationsEnabled()) {
            Log.w(TAG, "notifications are off: report for $chatId not posted")
            return false
        }
        val agent = Person.Builder().setName(title).setKey(chatId).setBot(true).build()
        val style = NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
            .setConversationTitle(title)
            .setGroupConversation(false)
            .addMessage(body, atMs.takeIf { it > 0 } ?: System.currentTimeMillis(), agent)
        val reply = NotificationCompat.Action.Builder(R.drawable.ic_car_mic, "Reply", service(chatId, CarReplyService.ACTION_REPLY, mutable = true))
            .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_REPLY)
            .setShowsUserInterface(false)
            .addRemoteInput(RemoteInput.Builder(CarMessaging.REPLY_KEY).setLabel("Reply to $title").build())
            .build()
        val read = NotificationCompat.Action.Builder(R.drawable.ic_car_mic, "Mark as read", service(chatId, CarReplyService.ACTION_MARK_READ, mutable = false))
            .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_MARK_AS_READ)
            .setShowsUserInterface(false)
            .build()
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.zi_microphone)
            .setContentTitle(header)
            .setContentText(Spoken.summary(body, 120))
            .setStyle(style)
            .addAction(reply)
            .addInvisibleAction(read)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setOnlyAlertOnce(false)
            // On the phone itself it just opens the app.
            .setContentIntent(
                PendingIntent.getActivity(context, chatId.hashCode(), Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
            )
            .build()
        return runCatching { manager.notify(TAG_NOTIFICATION, chatId.hashCode(), notification) }
            .onFailure { Log.w(TAG, "report notification failed", it) }
            .isSuccess
    }

    /** Fired by Android Auto: the reply (with its dictated text) or the mark-as-read. */
    private fun service(chatId: String, action: String, mutable: Boolean): PendingIntent =
        PendingIntent.getService(
            context,
            (chatId.hashCode() shl 2) + if (mutable) 2 else 3,
            CarReplyService.intent(context, action, chatId),
            PendingIntent.FLAG_UPDATE_CURRENT or if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE,
        )

    fun cancel(chatId: String) = manager.cancel(TAG_NOTIFICATION, chatId.hashCode())

    /** Opens the car app on [chatId] (and its microphone, for [reply]). */
    private fun open(chatId: String, reply: Boolean): PendingIntent {
        val intent = Intent(Intent.ACTION_VIEW)
            .setComponent(ComponentName(context, ZeronCarAppService::class.java))
            .setData(Uri.parse("zeron-car://chat/${Uri.encode(chatId)}" + if (reply) "/reply" else ""))
        return CarPendingIntent.getCarApp(context, (chatId.hashCode() shl 1) + if (reply) 1 else 0, intent, 0)
    }

    companion object {
        private const val TAG = "ZeronCar"
        private const val CHANNEL = "car-reports"
        private const val TAG_NOTIFICATION = "car-report"

        /** The chat a notification tap asks for, and whether to reply. */
        fun target(intent: Intent?): Pair<String, Boolean>? {
            val data = intent?.data ?: return null
            if (data.scheme != "zeron-car" || data.host != "chat") return null
            val segments = data.pathSegments
            val chatId = segments.firstOrNull() ?: return null
            return chatId to (segments.getOrNull(1) == "reply")
        }
    }
}
