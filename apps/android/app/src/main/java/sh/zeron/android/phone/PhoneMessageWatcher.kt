package sh.zeron.android.phone

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.provider.Telephony
import android.util.Log

/**
 * Chat (RCS) and picture messages don't announce themselves like an SMS does: Google Messages writes them
 * into the Telephony provider as MMS rows. This watches that table while the phone link is up, and passes
 * each NEW received message to the wake rule ([PhoneWake]). A row is only ever looked at once (a
 * watermark of the last row id seen), and a start never replays old ones: only rows from the last few
 * minutes wake the chat, the rest just move the watermark.
 */
object PhoneMessageWatcher {
    private const val TAG = "ZeronPhone"
    private const val FRESH_SECONDS = 15 * 60

    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var observer: ContentObserver? = null
    private var context: Context? = null

    @Synchronized
    fun start(context: Context) {
        if (observer != null) return
        val app = context.applicationContext
        if (app.checkSelfPermission(android.Manifest.permission.READ_SMS) != android.content.pm.PackageManager.PERMISSION_GRANTED) return
        this.context = app
        val t = HandlerThread("phone-messages").also { it.start() }
        val h = Handler(t.looper)
        val o = object : ContentObserver(h) {
            override fun onChange(selfChange: Boolean) = schedule()
        }
        thread = t
        handler = h
        observer = o
        app.contentResolver.registerContentObserver(Uri.parse("content://mms-sms/"), true, o)
        h.post { check(initial = true) }
    }

    @Synchronized
    fun stop() {
        val app = context ?: return
        observer?.let { app.contentResolver.unregisterContentObserver(it) }
        observer = null
        thread?.quitSafely()
        thread = null
        handler = null
    }

    private fun schedule() {
        val h = handler ?: return
        // The row lands before its text part does: look a moment later, once.
        h.removeCallbacksAndMessages(null)
        h.postDelayed({ check(initial = false) }, 1500)
    }

    private fun check(initial: Boolean) {
        val app = context ?: return
        val prefs = app.getSharedPreferences("phone-watch", Context.MODE_PRIVATE)
        val last = prefs.getLong("last-mms-id", -1)
        try {
            val sms = PhoneSms(app)
            val resolver = app.contentResolver
            if (last < 0) {
                // First run: start from now.
                val top = resolver.query(Telephony.Mms.CONTENT_URI, arrayOf("_id"), null, null, "_id DESC LIMIT 1")?.use { if (it.moveToFirst()) it.getLong(0) else 0L } ?: 0L
                prefs.edit().putLong("last-mms-id", top).apply()
                Log.i(TAG, "message watcher: starting after row $top")
                return
            }
            var newest = last
            val nowSeconds = System.currentTimeMillis() / 1000
            val rows = ArrayList<LongArray>()
            val kinds = HashMap<Long, String>()
            resolver.query(Telephony.Mms.CONTENT_URI, arrayOf("_id", "date", "msg_box", "tr_id", "sub"), "_id > ?", arrayOf(last.toString()), "_id ASC LIMIT 50")?.use { c ->
                while (c.moveToNext()) {
                    val id = c.getLong(0)
                    newest = maxOf(newest, id)
                    val incoming = c.getInt(2) == Telephony.Mms.MESSAGE_BOX_INBOX
                    val fresh = nowSeconds - c.getLong(1) <= FRESH_SECONDS
                    if (incoming && fresh) {
                        rows += longArrayOf(id)
                        kinds[id] = if ((c.getString(3) ?: "").startsWith("proto:") || (c.getString(4) ?: "").startsWith("proto:")) "rcs" else "mms"
                    }
                }
            }
            prefs.edit().putLong("last-mms-id", newest).apply()
            val settings = PhoneSettings(app)
            if (!settings.enabled || !settings.wakeEnabled) return
            for (row in rows) {
                val id = row[0].toString()
                var (text, attachments) = sms.parts(id)
                if (text.isEmpty() && attachments.isEmpty()) { Thread.sleep(1500); sms.parts(id).let { text = it.first; attachments = it.second } }
                // The row, its sender and its text parts are written one after another: give them a moment.
                var from = fromAddress(app, id)
                var waited = 0
                while (from == null && waited < 4) { Thread.sleep(1000); waited++; from = fromAddress(app, id) }
                if (from == null) { Log.i(TAG, "message watcher: row $id has no sender yet; skipped"); continue }
                val body = text.ifEmpty { if (attachments.isNotEmpty()) "[${attachments.joinToString(", ") { it.first }}]" else "" }
                if (body.isEmpty()) continue
                PhoneWake.message(app, from, body, kinds[row[0]] ?: "mms", null)
            }
        } catch (e: Exception) {
            Log.w(TAG, "message watcher failed", e)
        }
    }

    private fun fromAddress(context: Context, mmsId: String): String? =
        context.contentResolver.query(Uri.parse("content://mms/$mmsId/addr"), arrayOf("address", "type"), "type = 137", null, null)?.use {
            if (it.moveToFirst()) it.getString(0)?.takeIf { a -> a.isNotBlank() && a != "insert-address-token" } else null
        }
}
