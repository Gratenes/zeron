package sh.zeron.android.phone

import java.util.concurrent.TimeUnit

/** The user service Shizuku starts as the shell user. No Android context here: it is a bare process. */
class ShellService : IShellService.Stub() {
    override fun destroy() {
        System.exit(0)
    }

    override fun run(argv: Array<String>, timeoutMs: Int): String = exec(argv, null, timeoutMs)

    override fun runWithFile(argv: Array<String>, stdinPath: String, timeoutMs: Int): String = exec(argv, stdinPath, timeoutMs)

    private fun exec(argv: Array<String>, stdinPath: String?, timeoutMs: Int): String {
        return try {
            val process = ProcessBuilder(*argv).redirectErrorStream(true).start()
            if (stdinPath != null) {
                Thread {
                    runCatching { java.io.File(stdinPath).inputStream().use { i -> process.outputStream.use { o -> i.copyTo(o) } } }
                }.apply { start() }
            } else {
                runCatching { process.outputStream.close() }
            }
            val out = StringBuilder()
            val reader = Thread {
                process.inputStream.bufferedReader().use { r ->
                    val buf = CharArray(4096)
                    while (true) {
                        val n = r.read(buf)
                        if (n < 0) break
                        if (out.length < MAX) out.append(buf, 0, minOf(n, MAX - out.length))
                    }
                }
            }.apply { start() }
            val done = process.waitFor(timeoutMs.toLong(), TimeUnit.MILLISECONDS)
            if (!done) process.destroyForcibly()
            reader.join(2000)
            "${if (done) process.exitValue() else -1}\n$out"
        } catch (e: Exception) {
            "-2\n${e.javaClass.simpleName}: ${e.message}"
        }
    }

    private companion object {
        const val MAX = 64 * 1024
    }
}
