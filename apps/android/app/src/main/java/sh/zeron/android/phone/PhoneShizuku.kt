package sh.zeron.android.phone

import android.content.ComponentName
import android.content.Context
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.IBinder
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import rikka.shizuku.Shizuku

/**
 * The Shizuku client: Zeron asks the person's Shizuku (started by them, by wireless debugging or root) for a user
 * service that runs as the shell. Used only by typed tools (install, grant, force-stop, settings-get, the probe);
 * there is no raw shell for agents.
 */
object PhoneShizuku {
    const val REQUEST_CODE = 4201
    private const val PACKAGE = "moe.shizuku.privileged.api"

    class State(val installed: Boolean, val running: Boolean, val permitted: Boolean, val uid: Int?, val version: Int?)

    fun state(context: Context): State {
        val installed = runCatching { context.packageManager.getPackageInfo(PACKAGE, 0); true }.getOrDefault(false)
        val running = runCatching { Shizuku.pingBinder() }.getOrDefault(false)
        val permitted = running && runCatching { Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED }.getOrDefault(false)
        return State(installed, running, permitted, if (running) runCatching { Shizuku.getUid() }.getOrNull() else null, if (running) runCatching { Shizuku.getVersion() }.getOrNull() else null)
    }

    /** From the settings screen: asks Shizuku (its own dialog) to let Zeron use it. */
    fun requestPermission() {
        if (runCatching { Shizuku.pingBinder() }.getOrDefault(false) && !Shizuku.isPreV11()) runCatching { Shizuku.requestPermission(REQUEST_CODE) }
    }

    /** A clear reason this can't run now, or null if Shizuku is ready. */
    fun problem(context: Context): String? {
        val s = state(context)
        return when {
            !s.installed -> "Shizuku isn't installed on this phone: the person installs it (shizuku.rikka.app) and starts it, then allows Zeron in Zeron → Settings → Agent phone access → Shizuku."
            !s.running -> "Shizuku isn't running: the person has to start it (open Shizuku and start it with wireless debugging, or with root). It stops when the phone restarts."
            !s.permitted -> "Zeron isn't allowed to use Shizuku yet: the person taps Allow in Zeron → Settings → Agent phone access → Shizuku."
            else -> null
        }
    }

    @Volatile private var service: IShellService? = null
    @Volatile private var binding: CompletableFuture<IShellService>? = null

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val s = IShellService.Stub.asInterface(binder)
            service = s
            binding?.complete(s)
        }

        override fun onServiceDisconnected(name: ComponentName) {
            service = null
            binding = null
        }
    }

    private fun connect(context: Context): IShellService {
        service?.takeIf { it.asBinder().pingBinder() }?.let { return it }
        val future = synchronized(this) {
            binding?.takeIf { !it.isDone || service != null } ?: CompletableFuture<IShellService>().also {
                binding = it
                val args = Shizuku.UserServiceArgs(ComponentName(context.packageName, ShellService::class.java.name))
                    .processNameSuffix("shell").daemon(false).version(1)
                Shizuku.bindUserService(args, connection)
            }
        }
        return try {
            future.get(10, TimeUnit.SECONDS)
        } catch (e: Exception) {
            binding = null
            throw PhoneToolException("Shizuku didn't start Zeron's shell service in 10 s: is Shizuku still running? (${e.javaClass.simpleName})")
        }
    }

    class Result(val exit: Int, val output: String)

    /** Runs a fixed argv as the shell user. Typed tools build the argv: nothing here takes a shell string. */
    fun run(context: Context, argv: List<String>, timeoutMs: Int = 30_000): Result {
        problem(context)?.let { throw PhoneToolException(it) }
        val raw = connect(context.applicationContext).run(argv.toTypedArray(), timeoutMs)
        val nl = raw.indexOf('\n')
        return Result(raw.substring(0, if (nl < 0) raw.length else nl).trim().toIntOrNull() ?: -3, if (nl < 0) "" else raw.substring(nl + 1))
    }
}
