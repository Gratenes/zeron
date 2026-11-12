package sh.zeron.android.phone

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import sh.zeron.android.R

/**
 * Asks the person before something an agent asked for goes out (a text message): a heads-up
 * notification with the whole text and Send / Don't send. The call waits for the answer, up to
 * [WAIT_SECONDS], and no answer is a no.
 */
object PhoneConfirm {
    private const val CHANNEL = "phone-confirm"
    const val WAIT_SECONDS = 60L
    const val ACTION = "sh.zeron.android.PHONE_CONFIRM"
    private val pending = ConcurrentHashMap<Int, CompletableFuture<Boolean>>()
    private var next = 7300

    /** `true` only if the person said yes in time. Blocks the calling (tool) thread. */
    fun ask(context: Context, title: String, text: String): Boolean {
        val app = context.applicationContext
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(app, android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) throw PhoneToolException("Asking you first needs notifications: allow them in Zeron → Settings → Agent phone access, or switch the question off there")
        val manager = app.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "Agent questions", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "An agent asks before it sends a text message from your number"
                },
            )
        }
        val id = synchronized(this) { next++ }
        val answer = CompletableFuture<Boolean>()
        pending[id] = answer
        fun action(yes: Boolean) = PendingIntent.getBroadcast(
            app, id * 2 + if (yes) 1 else 0,
            Intent(ACTION).setPackage(app.packageName).putExtra("id", id).putExtra("yes", yes),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        manager.notify(
            id,
            NotificationCompat.Builder(app, CHANNEL)
                .setSmallIcon(R.drawable.zi_arrow_up)
                .setContentTitle(title)
                .setContentText(text)
                .setStyle(NotificationCompat.BigTextStyle().bigText(text))
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                .setAutoCancel(true)
                .setTimeoutAfter(WAIT_SECONDS * 1000)
                .addAction(0, "Send", action(true))
                .addAction(0, "Don't send", action(false))
                .build(),
        )
        return try {
            answer.get(WAIT_SECONDS, TimeUnit.SECONDS)
        } catch (_: Exception) {
            false
        } finally {
            pending.remove(id)
            manager.cancel(id)
        }
    }

    fun answer(id: Int, yes: Boolean) {
        pending[id]?.complete(yes)
    }

    fun denyAll() {
        pending.values.forEach { it.complete(false) }
    }
}

class PhoneConfirmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == PhoneConfirm.ACTION) PhoneConfirm.answer(intent.getIntExtra("id", -1), intent.getBooleanExtra("yes", false))
    }
}
