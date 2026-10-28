package sh.zeron.android.car

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.car.app.notification.CarAppExtender
import androidx.car.app.notification.CarPendingIntent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import sh.zeron.android.MainActivity
import sh.zeron.android.R

/**
 * A report as a car notification, for when the driver has left Zeron's
 * screen (for Maps, say): it heads up over whatever is showing, Open goes
 * back to that chat and Reply goes straight to the microphone. One per
 * chat; opening the chat clears it.
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

    fun post(report: Report) {
        if (!manager.areNotificationsEnabled()) {
            Log.w(TAG, "notifications are off: report for ${report.chatId} not posted")
            return
        }
        val title = when (report.kind) {
            ReportKind.Finished -> "${report.title} is done"
            ReportKind.NeedsInput -> "${report.title} needs you"
            ReportKind.Failed -> "${report.title} failed"
        }
        val text = Spoken.summary(report.text, 120).ifEmpty { report.spoken }
        val id = report.chatId.hashCode()
        val car = CarAppExtender.Builder()
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_car_mic)
            .setImportance(NotificationManager.IMPORTANCE_HIGH)
            .setContentIntent(open(report.chatId, reply = false))
            .addAction(R.drawable.ic_car_mic, if (report.kind == ReportKind.NeedsInput) "Answer" else "Reply", open(report.chatId, reply = true))
            .build()
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.zi_microphone)
            .setContentTitle(title)
            .setContentText(text)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setOnlyAlertOnce(false)
            // On the phone itself it just opens the app.
            .setContentIntent(
                PendingIntent.getActivity(context, id, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
            )
            .extend(car)
            .build()
        runCatching { manager.notify(TAG_NOTIFICATION, id, notification) }
            .onFailure { Log.w(TAG, "report notification failed", it) }
    }

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
