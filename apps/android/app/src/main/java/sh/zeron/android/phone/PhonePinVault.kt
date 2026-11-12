package sh.zeron.android.phone

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The phone's PIN, kept so an agent can unlock the phone when the person has allowed it. The PIN is
 * encrypted with an AES-256-GCM key made in the Android Keystore (in StrongBox hardware when the phone has
 * it): the key never leaves the secure hardware or this app, needs no fingerprint (the phone is locked when it
 * is used), and the encrypted PIN lives in the app's private storage. The PIN is never logged, never sent to
 * an agent, and never leaves the phone.
 */
object PhonePinVault {
    private const val ALIAS = "zeron-phone-pin"
    private const val PROVIDER = "AndroidKeyStore"
    private const val PREFS = "phone-pin"

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun has(context: Context): Boolean = prefs(context).contains("cipher") && keyOrNull() != null

    /** `"StrongBox"`, `"secure hardware"` or `"software"`: where the key lives, for the person to read. */
    fun protection(context: Context): String = prefs(context).getString("protection", "") ?: ""

    fun store(context: Context, pin: String) {
        require(pin.length in 4..16 && pin.all { it.isDigit() }) { "a PIN is 4 to 16 digits" }
        forget(context)
        val (key, protection) = newKey(context)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }
        val bytes = cipher.doFinal(pin.toByteArray(Charsets.UTF_8))
        prefs(context).edit()
            .putString("cipher", Base64.encodeToString(bytes, Base64.NO_WRAP))
            .putString("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .putString("protection", protection)
            .apply()
    }

    /** The PIN, or null when none is stored (or its key is gone). */
    fun load(context: Context): String? {
        val p = prefs(context)
        val cipherText = p.getString("cipher", null) ?: return null
        val iv = p.getString("iv", null) ?: return null
        val key = keyOrNull() ?: return null
        return runCatching {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, Base64.decode(iv, Base64.NO_WRAP)))
            }
            String(cipher.doFinal(Base64.decode(cipherText, Base64.NO_WRAP)), Charsets.UTF_8)
        }.getOrNull()
    }

    fun forget(context: Context) {
        prefs(context).edit().clear().apply()
        runCatching { KeyStore.getInstance(PROVIDER).apply { load(null) }.deleteEntry(ALIAS) }
    }

    private fun keyOrNull(): SecretKey? = runCatching {
        (KeyStore.getInstance(PROVIDER).apply { load(null) }.getKey(ALIAS, null)) as? SecretKey
    }.getOrNull()

    private fun newKey(context: Context): Pair<SecretKey, String> {
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(false)
            .apply { if (strongBox && Build.VERSION.SDK_INT >= 28) setIsStrongBoxBacked(true) }
            .build()
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        return try {
            generator.init(spec(strongBox = true))
            generator.generateKey() to "StrongBox"
        } catch (_: StrongBoxUnavailableException) {
            generator.init(spec(strongBox = false))
            val key = generator.generateKey()
            val info = runCatching {
                javax.crypto.SecretKeyFactory.getInstance(key.algorithm, PROVIDER).getKeySpec(key, android.security.keystore.KeyInfo::class.java) as android.security.keystore.KeyInfo
            }.getOrNull()
            key to if (info?.isInsideSecureHardware == true) "secure hardware" else "software"
        } catch (_: Exception) {
            generator.init(spec(strongBox = false))
            generator.generateKey() to "software"
        }
    }
}
