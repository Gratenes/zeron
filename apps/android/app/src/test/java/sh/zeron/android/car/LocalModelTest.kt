package sh.zeron.android.car

import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.net.InetSocketAddress
import java.security.MessageDigest

class LocalModelTest {
    @get:Rule val tmp = TemporaryFolder()

    private lateinit var server: HttpServer
    private val bodies = mapOf(
        "a.bin" to ByteArray(3_000_000) { (it * 7).toByte() },
        "b.bin" to ByteArray(1_000) { it.toByte() },
    )
    private val ranges = mutableListOf<String?>()
    private var corrupt = false
    private val base get() = "http://127.0.0.1:${server.address.port}/"

    private fun sha(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    private val files get() = bodies.map { (name, bytes) -> ModelFile(name, bytes.size.toLong(), sha(bytes)) }

    @Before
    fun serve() {
        server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/") { ex ->
            val bytes = bodies[ex.requestURI.path.trimStart('/')]
            if (bytes == null) {
                ex.sendResponseHeaders(404, -1)
            } else {
                val range = ex.requestHeaders.getFirst("Range")
                ranges += range
                val from = range?.removePrefix("bytes=")?.removeSuffix("-")?.toInt() ?: 0
                val body = bytes.copyOfRange(from, bytes.size).also { if (corrupt && it.isNotEmpty()) it[it.size / 2] = 0x55 }
                ex.sendResponseHeaders(if (range != null) 206 else 200, body.size.toLong())
                ex.responseBody.use { it.write(body) }
            }
            ex.close()
        }
        server.start()
    }

    @After
    fun stop() = server.stop(0)

    private fun model(dir: File = tmp.newFolder("model"), revision: String = "r1") =
        LocalModel(dir, files, base, revision, httpSource())

    private fun httpSource() = ModelSource { url, from ->
        (java.net.URL(url).openConnection() as java.net.HttpURLConnection).apply {
            if (from > 0) setRequestProperty("Range", "bytes=$from-")
        }
    }

    @Test
    fun downloadsVerifiesAndBecomesReady() = runBlocking {
        val m = model()
        assertFalse(m.isReady())
        assertEquals(ModelState.Missing, m.state.value)
        m.download()
        assertTrue(m.isReady())
        assertEquals(ModelState.Ready, m.state.value)
        assertTrue(File(m.dir, "a.bin").readBytes().contentEquals(bodies.getValue("a.bin")))
        assertFalse("staging is cleaned up", File(m.dir, ".staging").exists())
    }

    @Test
    fun reportsProgressUpToTheTotal() = runBlocking {
        val m = model()
        val seen = mutableListOf<Long>()
        val job = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Unconfined).launch {
            m.state.collect { s -> if (s is ModelState.Downloading) seen += s.done else if (s == ModelState.Ready) seen += m.totalBytes }
        }
        m.download()
        job.cancel()
        assertTrue(seen.isNotEmpty())
        assertEquals(seen.sorted(), seen)
        assertEquals(m.totalBytes, seen.last())
    }

    @Test
    fun aCorruptDownloadIsRefusedAndNeverMovedIntoPlace() = runBlocking {
        corrupt = true
        val m = model()
        try {
            m.download()
            fail("a checksum mismatch must fail")
        } catch (e: java.io.IOException) {
            assertTrue(e.message!!.contains("checksum"))
        }
        assertFalse(m.isReady())
        assertTrue(m.state.value is ModelState.Failed)
        assertFalse(File(m.dir, "a.bin").exists())
    }

    @Test
    fun anInterruptedDownloadResumesFromItsPartFile() = runBlocking {
        val dir = tmp.newFolder("resume")
        File(dir, ".staging").mkdirs()
        File(dir, ".staging/a.bin.part").writeBytes(bodies.getValue("a.bin").copyOfRange(0, 1_000_000))
        val m = model(dir)
        m.download()
        assertTrue(m.isReady())
        assertTrue("asked for the rest only: $ranges", ranges.contains("bytes=1000000-"))
        assertTrue(File(dir, "a.bin").readBytes().contentEquals(bodies.getValue("a.bin")))
    }

    @Test
    fun aMissingFileFailsWithTheStatus() = runBlocking {
        val m = LocalModel(tmp.newFolder("m"), listOf(ModelFile("nope.bin", 5, "00")), base, "r1", httpSource())
        try {
            m.download()
            fail()
        } catch (e: java.io.IOException) {
            assertTrue(e.message!!.contains("404"))
        }
    }

    @Test
    fun aNewRevisionIsNotReadyUntilDownloadedAgain() = runBlocking {
        val dir = tmp.newFolder("rev")
        model(dir, "r1").download()
        assertTrue(model(dir, "r1").isReady())
        assertFalse(model(dir, "r2").isReady())
    }

    @Test
    fun aTruncatedFileIsNotReady() = runBlocking {
        val m = model()
        m.download()
        File(m.dir, "a.bin").writeBytes(ByteArray(10))
        assertFalse(model(m.dir).isReady())
    }

    @Test
    fun removeDeletesEverything() = runBlocking {
        val m = model()
        m.download()
        m.remove()
        assertFalse(m.dir.exists())
        assertEquals(ModelState.Missing, m.state.value)
    }

    @Test
    fun theShippedManifestPinsEveryFileByHash() {
        assertEquals(4, ParakeetUnified.FILES.size)
        assertTrue(ParakeetUnified.FILES.all { it.sha256.matches(Regex("[0-9a-f]{64}")) && it.size > 0 })
        assertTrue(ParakeetUnified.REVISION.matches(Regex("[0-9a-f]{40}")))
        assertTrue(ParakeetUnified.BASE_URL.contains(ParakeetUnified.REVISION))
        // The encoder's weights are the bulk of it: about 654 MB in all.
        assertEquals(663_344_373L, ParakeetUnified.FILES.sumOf { it.size })
    }
}

