package sh.zeron.android.phone

/**
 * Plain-language states for a text the phone sent, from what Android reports, so an agent never has to guess
 * whether "ok" meant "arrived". Pure functions, tested without Android.
 */
object SendReport {
    /** A message's state in the provider: `content://sms` `type`, or `content://mms` `msg_box` (same numbers). */
    fun providerState(box: Int): String = when (box) {
        2 -> "sent"
        4 -> "queued" // outbox: waiting to go
        6 -> "queued"
        5 -> "failed"
        else -> "other"
    }

    /** A delivery report's status (`SmsMessage.getStatus()`): 0 complete, 32..63 still trying, 64+ failed. */
    fun deliveryOf(status: Int): String = when {
        status == 0 -> "delivered"
        status in 32..63 -> "pending"
        else -> "failed"
    }

    /** True only when the network itself said it reached the other phone. Nothing else counts. */
    fun confirmed(delivery: String): Boolean = delivery == "delivered"

    fun note(transport: String, delivery: String, provider: String?): String = when {
        delivery == "delivered" -> "Delivered: the carrier reported it reached the other phone."
        delivery == "failed" -> "FAILED: the carrier reported it was not delivered. Tell the person; do not assume the recipient has it."
        provider == "failed" -> "FAILED: Google Messages shows this message as failed to send. Tell the person; do not assume the recipient has it."
        provider == "queued" -> "NOT SENT YET: Google Messages still has it queued (no signal, or it is retrying). The person has been told if it does not go."
        transport == "sms" -> "Handed to the carrier; no delivery report yet. This is NOT confirmation the recipient has it: check phone_sms_read later, and tell the person if it matters."
        provider == "sent" -> "Sent from Google Messages (it shows as sent). Chat messages give no delivery receipt here, so this is not proof the recipient has read it."
        else -> "Not yet seen in the message database: check with phone_sms_read before assuming it went."
    }
}
