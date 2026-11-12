package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Starts the phone link from a push. The push provider's service calls [PhoneLink.onWake] the
 * same way; this receiver is how a test or `adb shell am broadcast` stands in for it (debug
 * builds only: it is declared in the debug manifest).
 */
class PhoneWakeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        PhoneLink.onWake(context, intent.getStringExtra("reason") ?: "broadcast")
    }
}
