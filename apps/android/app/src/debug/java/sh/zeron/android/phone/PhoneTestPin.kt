package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug builds: `am broadcast -n sh.zeron.android/.phone.PhoneTestPinReceiver -a sh.zeron.android.TEST_PIN --es pin 2468` stores a PIN and turns agent unlock on. */
class PhoneTestPinReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pin = intent.getStringExtra("pin") ?: return
        PhonePinVault.store(context, pin)
        PhoneSettings(context).apply { unlockEnabled = true; unlockFailed = false }
        android.util.Log.i("ZeronPhone", "TEST pin stored (${PhonePinVault.protection(context)})")
    }
}
