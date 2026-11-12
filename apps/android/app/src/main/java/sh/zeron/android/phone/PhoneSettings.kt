package sh.zeron.android.phone

import android.content.Context

/**
 * What the person lets agents do on this phone. Off by default: nothing
 * works until the master switch is on, and each tool group has its own.
 * Checked in [PhoneTools] on the phone, never trusted to the caller.
 */
class PhoneSettings(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("phone-access", Context.MODE_PRIVATE)

    var enabled: Boolean
        get() = prefs.getBoolean("enabled", false)
        set(value) = prefs.edit().putBoolean("enabled", value).apply()

    fun groupOn(group: Group): Boolean = prefs.getBoolean("group-${group.key}", group.defaultOn)

    fun setGroup(group: Group, on: Boolean) = prefs.edit().putBoolean("group-${group.key}", on).apply()

    /** The SMS-send confirmation: a setting either way (the default is the person's to pick). */
    var confirmSmsSend: Boolean
        get() = prefs.getBoolean("confirm-sms-send", true)
        set(value) = prefs.edit().putBoolean("confirm-sms-send", value).apply()

    enum class Group(val key: String, val label: String, val defaultOn: Boolean) {
        SCREEN_READ("screen-read", "Read the screen", true),
        CONTROL("control", "Tap, type and swipe", true),
        SMS_READ("sms-read", "Read text messages", true),
        SMS_SEND("sms-send", "Send text messages", true),
    }
}
