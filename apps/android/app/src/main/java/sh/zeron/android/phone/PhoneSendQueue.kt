package sh.zeron.android.phone

import android.app.Notification
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
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONArray
import org.json.JSONObject
import sh.zeron.android.MainActivity
import sh.zeron.android.R

/**
 * A text that needs the phone's screen (Google Messages' compose screen) while the person is using the phone is not
 * refused: it waits here. It is retried when the person has been idle [IDLE_MS] (or the screen is off or locked and
 * unlocking is allowed), until [LIFETIME_MS]. Meanwhile the person has a notification — "Zeron wants to text X" with
 * Send now / Edit / Cancel — and the agent is told it is queued, then reads the outcome with `phone_sms_queue`.
 * Kept in memory only: if Zeron's process is killed, queued texts are lost (the person was notified of each).
 */
object PhoneSendQueue {
    const val IDLE_MS = 25_000L
    const val LIFETIME_MS = 10 * 60_000L
    private const val TICK_MS = 2_000L
    private const val CHANNEL = "phone-queued-texts"
    const val ACTION = "sh.zeron.android.PHONE_QUEUE"

    class Entry(
        val id: Int, val number: String, val name: String?, val preview: String, val reason: String,
        val wantRcs: Boolean, val created: Long, val run: (Boolean) -> JSONObject,
    ) {
        @Volatile var state: String = "queued" // queued, sending, sent, failed, expired, cancelled, handed
        @Volatile var detail: String? = null
        @Volatile var result: JSONObject? = null
        @Volatile var finished: Long = 0
    }

    private val entries = ConcurrentHashMap<Int, Entry>()
    private val ids = AtomicInteger(1)
    @Volatile private var worker: Thread? = null

    /** Queues [run] (called with `force` true when the person tapped Send now) and tells the agent so. */
    fun enqueue(context: Context, number: String, name: String?, preview: String, reason: String, wantRcs: Boolean, run: (Boolean) -> JSONObject): JSONObject {
        val app = context.applicationContext
        val e = Entry(ids.getAndIncrement(), number, name, preview, reason, wantRcs, System.currentTimeMillis(), run)
        entries[e.id] = e
        PhoneActionLog.add(app, "sms_send", "queued a text to ${name ?: number}: the person is using the phone", true, null)
        notifyQueued(app, e)
        startWorker(app)
        return JSONObject().put("ok", true).put("queued", true).put("state", "queued").put("queueId", e.id).put("to", number)
            .put("sent", false).put("deliveryConfirmed", false)
            .put("note", "QUEUED, NOT SENT YET: the person is using the phone, so the text waits for it to be free (it is retried when they have been idle ${IDLE_MS / 1000} s or the screen is off, for up to ${LIFETIME_MS / 60_000} min). They have a notification with Send now / Edit / Cancel. Read the outcome with phone_sms_queue {id: ${e.id}}; do not resend meanwhile, and do not tell anyone it went.")
    }

    /** What phone_sms_queue says: one entry (by id) or every recent one. */
    fun status(id: Int?): JSONObject {
        val list = JSONArray()
        entries.values.filter { id == null || it.id == id }.sortedBy { it.id }.forEach { e ->
            list.put(JSONObject().put("queueId", e.id).put("to", e.number).put("state", e.state).put("detail", e.detail ?: JSONObject.NULL)
                .put("preview", e.preview.take(60)).put("ageSeconds", (System.currentTimeMillis() - e.created) / 1000).put("result", e.result ?: JSONObject.NULL))
        }
        if (id != null && list.length() == 0) throw PhoneToolException("no queued text $id (it may be older than the phone remembers, or Zeron restarted)")
        return JSONObject().put("queue", list).put("note", "state: queued/sending (not sent yet), sent (result has providerState; chats give no delivery receipt), failed, expired (gave up after ${LIFETIME_MS / 60_000} min), cancelled or handed (the person took it into Messages to edit).")
    }

    private fun startWorker(app: Context) {
        synchronized(this) {
            if (worker?.isAlive == true) return
            worker = Thread {
                while (true) {
                    Thread.sleep(TICK_MS)
                    val pending = entries.values.filter { it.state == "queued" }
                    if (pending.isEmpty()) {
                        // Keep finished entries for a while so the agent can read them.
                        entries.values.filter { it.finished != 0L && System.currentTimeMillis() - it.finished > 30 * 60_000L }.forEach { entries.remove(it.id) }
                        if (entries.values.none { it.state == "queued" || it.state == "sending" }) { worker = null; return@Thread }
                        continue
                    }
                    for (e in pending.sortedBy { it.id }) {
                        if (e.state != "queued") continue
                        when (QueuePolicy.decide(System.currentTimeMillis(), e.created, PhoneUnlock.personActiveWithin(IDLE_MS), LIFETIME_MS)) {
                            QueuePolicy.Decision.EXPIRED -> finish(app, e, "expired", "the phone was not free for ${LIFETIME_MS / 60_000} minutes", null)
                            QueuePolicy.Decision.WAIT -> Unit
                            QueuePolicy.Decision.GO -> { attempt(app, e, force = false); break } // one at a time: they share the screen
                        }
                    }
                }
            }.apply { isDaemon = true; name = "zeron-send-queue" }.also { it.start() }
        }
    }

    private fun attempt(app: Context, e: Entry, force: Boolean) {
        synchronized(e) {
            if (e.state != "queued") return
            e.state = "sending"
        }
        try {
            val result = e.run(force)
            finish(app, e, "sent", null, result)
        } catch (busy: PhoneBusyException) {
            e.state = "queued" // not yet: the person is back on the phone
        } catch (t: PhoneToolException) {
            finish(app, e, "failed", t.message, null)
        } catch (t: Exception) {
            finish(app, e, "failed", t.message ?: t.javaClass.simpleName, null)
        }
    }

    private fun finish(app: Context, e: Entry, state: String, detail: String?, result: JSONObject?) {
        e.state = state; e.detail = detail; e.result = result; e.finished = System.currentTimeMillis()
        val who = e.name ?: e.number
        PhoneActionLog.add(app, "sms_send", "queued text to $who: $state${detail?.let { " ($it)" } ?: ""}", state == "sent", null)
        val manager = app.getSystemService(NotificationManager::class.java)
        when (state) {
            "sent" -> post(app, e, "Text to $who sent", "\"${e.preview.take(80)}\"", actions = false, quiet = true)
            "failed" -> post(app, e, "Text to $who failed", "${detail.orEmpty()}: \"${e.preview.take(60)}\"", actions = false, quiet = false)
            "expired" -> post(app, e, "Text to $who was not sent", "The phone wasn't free for ${LIFETIME_MS / 60_000} minutes: \"${e.preview.take(60)}\"", actions = false, quiet = false)
            else -> manager.cancel(notificationId(e))
        }
    }

    private fun notificationId(e: Entry) = 7400 + e.id % 500

    private fun notifyQueued(app: Context, e: Entry) =
        post(app, e, "Zeron wants to text ${e.name ?: e.number}", "\"${e.preview.take(120)}\" — waiting for you to finish with the phone", actions = true, quiet = false)

    private fun post(app: Context, e: Entry, title: String, text: String, actions: Boolean, quiet: Boolean) {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(app, android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val manager = app.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "Texts waiting to send", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "A text an agent wants to send while you are using the phone: send it now, edit it, or cancel"
            })
        }
        fun action(what: String, code: Int) = PendingIntent.getBroadcast(app, e.id * 10 + code,
            Intent(ACTION).setPackage(app.packageName).putExtra("id", e.id).putExtra("do", what), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val b = NotificationCompat.Builder(app, CHANNEL)
            .setSmallIcon(R.drawable.zi_arrow_up).setContentTitle(title).setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setPriority(if (quiet) NotificationCompat.PRIORITY_LOW else NotificationCompat.PRIORITY_HIGH)
            .setCategory(Notification.CATEGORY_MESSAGE).setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(app, 0, Intent(app, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE))
        if (actions) {
            b.setOngoing(false).setTimeoutAfter(LIFETIME_MS)
            b.addAction(0, "Send now", action("now", 1)).addAction(0, "Edit", action("edit", 2)).addAction(0, "Cancel", action("cancel", 3))
        } else {
            b.setTimeoutAfter(if (quiet) 10_000L else 6 * 60 * 60_000L)
        }
        manager.notify(notificationId(e), b.build())
    }

    /** A tap on one of the notification's buttons. */
    fun act(context: Context, id: Int, what: String) {
        val app = context.applicationContext
        val e = entries[id] ?: return
        when (what) {
            "cancel" -> if (e.state == "queued") { finish(app, e, "cancelled", "the person cancelled it", null) }
            "edit" -> if (e.state == "queued") {
                // Messages opens on the conversation with the text in the compose box: the person edits and sends.
                finish(app, e, "handed", "the person took it into Google Messages to edit and send", null)
                val intent = Intent(Intent.ACTION_SENDTO, android.net.Uri.parse("smsto:" + android.net.Uri.encode(e.number)))
                    .setPackage("com.google.android.apps.messaging").putExtra("sms_body", e.preview).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                runCatching { app.startActivity(intent) }
            }
            "now" -> if (e.state == "queued") Thread { attempt(app, e, force = true) }.apply { isDaemon = true }.start()
        }
    }
}

class PhoneQueueReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != PhoneSendQueue.ACTION) return
        PhoneSendQueue.act(context, intent.getIntExtra("id", -1), intent.getStringExtra("do").orEmpty())
    }
}
