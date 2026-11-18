package sh.zeron.android.phone

/** A tool that can't do what was asked: the message is what the agent reads. */
open class PhoneToolException(message: String) : Exception(message)

/** The person is using the phone: what needs its screen can wait for them to be done (see [PhoneSendQueue]). */
class PhoneBusyException(message: String) : PhoneToolException(message)
