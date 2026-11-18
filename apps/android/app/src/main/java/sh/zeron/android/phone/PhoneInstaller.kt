package sh.zeron.android.phone

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageInstaller
import androidx.core.content.ContextCompat
import java.io.File
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/**
 * The install without Shizuku: an Android PackageInstaller session. Android asks the person to confirm on screen
 * (the system's installer is one of the apps agents can never see or tap), so this waits for their tap.
 */
class PhoneInstaller(private val context: Context) {
    /** (installed, message). */
    fun install(file: File, pkg: String): Pair<Boolean, String> {
        val app = context.applicationContext
        val pm = app.packageManager
        if (!pm.canRequestPackageInstalls()) {
            return false to "Zeron isn't allowed to install apps yet: the person turns on “Install unknown apps” for Zeron in Android Settings (or starts Shizuku and allows it)."
        }
        val installer = pm.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply { setAppPackageName(pkg) }
        val sessionId = installer.createSession(params)
        val action = "sh.zeron.android.INSTALL_RESULT.$sessionId"
        val done = CompletableFuture<Pair<Boolean, String>>()
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                when (val status = i.getIntExtra(PackageInstaller.EXTRA_STATUS, -999)) {
                    PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                        @Suppress("DEPRECATION")
                        val prompt = i.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
                        prompt?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        // From the accessibility service if it is on (it may start activities from the background).
                        runCatching { (PhoneAccessibilityService.instance ?: app).startActivity(prompt) }
                    }
                    PackageInstaller.STATUS_SUCCESS -> done.complete(true to "installed (the person confirmed it)")
                    else -> done.complete(false to (i.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "status $status"))
                }
            }
        }
        ContextCompat.registerReceiver(app, receiver, IntentFilter(action), ContextCompat.RECEIVER_NOT_EXPORTED)
        try {
            installer.openSession(sessionId).use { session ->
                file.inputStream().use { input -> session.openWrite("base.apk", 0, file.length()).use { out -> input.copyTo(out); session.fsync(out) } }
                val pending = PendingIntent.getBroadcast(app, sessionId, Intent(action).setPackage(app.packageName), PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
                session.commit(pending.intentSender)
            }
            PhoneUnlock.ours(60_000)
            return try { done.get(90, TimeUnit.SECONDS) } catch (_: Exception) { false to "the person didn't confirm the install in 90 s" }
        } catch (e: Exception) {
            runCatching { installer.abandonSession(sessionId) }
            return false to "${e.javaClass.simpleName}: ${e.message}"
        } finally {
            runCatching { app.unregisterReceiver(receiver) }
        }
    }
}
