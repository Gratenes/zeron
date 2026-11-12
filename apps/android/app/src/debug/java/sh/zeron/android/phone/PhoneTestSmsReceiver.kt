package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug builds: `am broadcast -a sh.zeron.android.TEST_SMS --es from N --es body T` stands in for an incoming text. */
class PhoneTestSmsReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        PhoneSmsReceiver().handle(context, intent.getStringExtra("from") ?: return, intent.getStringExtra("body") ?: "", goAsync())
    }
}
