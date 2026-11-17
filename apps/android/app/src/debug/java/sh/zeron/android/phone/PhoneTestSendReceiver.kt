package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/** Debug builds: `am broadcast -a sh.zeron.android.TEST_SEND --es to N --es text T --es via sms|auto|rcs` runs phone_sms_send; the result is in logcat tag ZeronSend. */
class PhoneTestSendReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        Thread {
            val out = runCatching {
                val sms = PhoneSms(context)
                when (intent.getStringExtra("tool")) {
                    "attachment" -> return@runCatching sms.attachment(intent.getStringExtra("message_id") ?: "", intent.getStringExtra("part_id") ?: "").toString().take(600)
                    "files" -> {
                        val f = PhoneFiles(context)
                        val path = intent.getStringExtra("path") ?: ""
                        return@runCatching when (intent.getStringExtra("op")) {
                            "list" -> f.list(path, null).toString()
                            "stat" -> f.stat(path).toString()
                            "hash" -> f.hash(path).toString()
                            "read" -> f.read(path, intent.getLongExtra("offset", 0), intent.getIntExtra("len", 2400)).toString()
                            "write" -> f.writePart(intent.getStringExtra("id") ?: "t", path, intent.getLongExtra("offset", 0), android.util.Base64.decode(intent.getStringExtra("data") ?: "", android.util.Base64.DEFAULT), intent.getBooleanExtra("final", true), intent.getStringExtra("sha"), intent.getBooleanExtra("overwrite", false), null).toString()
                            else -> "bad op"
                        }
                    }
                    "read" -> return@runCatching sms.read(intent.getStringExtra("thread_id"), null, 10, null).toString()
                }
                sms.send(intent.getStringExtra("to"), null, intent.getStringExtra("text") ?: "test", intent.getStringExtra("via") ?: "auto", null).toString()
            }.getOrElse { "ERROR ${it.message}" }
            Log.i("ZeronSend", out.take(3800))
            pending.finish()
        }.start()
    }
}
