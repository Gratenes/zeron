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
                PhoneSms(context).send(intent.getStringExtra("to"), null, intent.getStringExtra("text") ?: "test", intent.getStringExtra("via") ?: "auto", null).toString()
            }.getOrElse { "ERROR ${it.message}" }
            Log.i("ZeronSend", out)
            pending.finish()
        }.start()
    }
}
