package sh.zeron.android.car

import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.util.Log
import androidx.core.app.RemoteInput
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import sh.zeron.android.ZeronApplication
import sh.zeron.android.core.LaunchOptions

/**
 * The background half of a report's messaging notification: Android Auto (and
 * so Assistant) fires it with the driver's dictated reply, or when they mark
 * the report read. No screen: Android Auto blocks apps whose reply action
 * opens one. Works with the car app closed; it starts the app's core itself.
 */
class CarReplyService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var running = 0

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val chatId = intent?.getStringExtra(EXTRA_CHAT)
        val app = application as ZeronApplication
        val notifications = CarNotifications(this)
        running++
        scope.launch {
            try {
                when (intent?.action) {
                    ACTION_REPLY -> reply(app, notifications, chatId, RemoteInput.getResultsFromIntent(intent)?.getCharSequence(CarMessaging.REPLY_KEY))
                    ACTION_MARK_READ -> if (chatId != null) {
                        app.model.car.markSeen(chatId)
                        notifications.cancel(chatId)
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "report action failed", e)
            } finally {
                if (--running == 0) stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    private suspend fun reply(app: ZeronApplication, notifications: CarNotifications, chatId: String?, text: CharSequence?) {
        val model = app.model
        // Android Auto may fire this with the app's process just started.
        if (model.client.value == null) model.boot(LaunchOptions())
        withTimeoutOrNull(CORE_WAIT) { model.client.first { it != null } }
        when (val result = CarMessaging.reply(chatId, text) { id, said -> model.car.reply(id, said) }) {
            is CarMessaging.Result.Sent -> {
                // Its next report reaches the driver even with Zeron off screen.
                model.car.follow(result.chatId)
                notifications.cancel(result.chatId)
            }
            is CarMessaging.Result.Failed -> if (chatId != null) notifications.failed(chatId, model.row(chatId)?.title ?: "Zeron", result.message)
            CarMessaging.Result.Nothing -> Unit
        }
    }

    override fun onDestroy() {
        scope.cancel()
    }

    companion object {
        private const val TAG = "ZeronCar"
        private const val CORE_WAIT = 20_000L
        const val ACTION_REPLY = "sh.zeron.android.car.REPLY"
        const val ACTION_MARK_READ = "sh.zeron.android.car.MARK_READ"
        const val EXTRA_CHAT = "chatId"

        fun intent(context: Context, action: String, chatId: String): Intent =
            Intent(context, CarReplyService::class.java).setAction(action).putExtra(EXTRA_CHAT, chatId)
    }
}
