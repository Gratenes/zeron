package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Telephony

/** After a reboot or an update, bring the phone link back if the person has it on. */
class PhoneBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (!PhoneSettings(context).enabled) return
        PhoneLink.onWake(context, intent.action ?: "boot")
    }
}

/**
 * A new text message. It always wakes the app (the system starts this receiver); what it does with
 * the text is the person's rule ([PhoneWake]): sent into the chat they chose if the sender is on their
 * list, otherwise only noted in the log. (Chat and picture messages arrive through [PhoneMessageWatcher].)
 */
class PhoneSmsReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Telephony.Sms.Intents.SMS_RECEIVED_ACTION) return
        val parts = Telephony.Sms.Intents.getMessagesFromIntent(intent) ?: return
        val from = parts.firstOrNull()?.originatingAddress ?: return
        handle(context, from, parts.joinToString("") { it.messageBody ?: "" }, goAsync())
    }

    /** A text from [from] arrived: apply the person's wake rule. */
    fun handle(context: Context, from: String, body: String, pending: PendingResult?) =
        PhoneWake.message(context, from, body, "sms", pending)

    companion object {
        fun senderAllowed(list: String, from: String): Boolean = PhoneWake.senderAllowed(list, from)
    }
}
