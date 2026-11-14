package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalListeningTest {
    private val speech = ByteArray(512) { i -> if (i % 2 == 0) 0 else 0x20 } // 16 ms, loud
    private val quiet = ByteArray(512)

    /** A microphone playing back [script] (each entry one 16 ms buffer), then end of stream. */
    private class Mic(private val script: List<ByteArray>) : AudioStream {
        var next = 0
        var stopped = false
        override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
            if (next >= script.size) return -1
            val chunk = script[next++]
            System.arraycopy(chunk, 0, buffer, offset, chunk.size)
            return chunk.size
        }

        override fun stop() {
            stopped = true
        }
    }

    private class FakeSession(val words: List<String> = emptyList(), val final: String = "", val failOnFeed: Int = -1) : AsrSession {
        var fed = 0
        var bytes = 0
        var finished = false
        override fun feed(pcm: ByteArray, length: Int): String? {
            if (fed == failOnFeed) error("boom")
            bytes += length
            return words.getOrNull(fed++)
        }

        override fun finish(): String {
            finished = true
            return final
        }

        override fun close() = Unit
    }

    private fun script(speechMs: Int, quietMs: Int) =
        List(200 / 16) { quiet } + List(speechMs / 16) { speech } + List(quietMs / 16) { quiet }

    @Test
    fun streamsPartialsThenTheOfflineTextEndsAtTheEndpoint() {
        val session = FakeSession(words = listOf("send", "send the", "send the patch"), final = "Send the patch.")
        val partials = mutableListOf<String>()
        val mic = Mic(script(speechMs = 1_000, quietMs = 3_000))
        val result = LocalListening(session).run(mic, partials::add) { false }
        assertEquals(LocalResult.Text("Send the patch."), result)
        assertEquals(listOf("send", "send the", "send the patch"), partials)
        assertTrue(session.finished)
        assertTrue("the microphone is released", mic.stopped)
        // It stopped reading at the endpoint (2.2 s of quiet), not at the end of the script.
        assertTrue(mic.next < script(1_000, 3_000).size)
    }

    @Test
    fun aPauseShorterThanTheEndpointKeepsListening() {
        val session = FakeSession(final = "one two")
        val mic = Mic(script(500, 1_500) + script(500, 3_000))
        LocalListening(session).run(mic, {}) { false }
        assertTrue("heard the second half too", session.bytes > 1_000 * 32 + 500 * 32)
    }

    @Test
    fun theStreamEndingEndsTheUtterance() {
        val session = FakeSession(final = "hmm")
        val result = LocalListening(session).run(Mic(script(500, 0)), {}) { false }
        assertEquals(LocalResult.Text("hmm"), result)
    }

    @Test
    fun anEmptyFinalFallsBackToTheLastPartial() {
        val session = FakeSession(words = listOf("hello there"), final = "")
        val result = LocalListening(session).run(Mic(script(1_000, 3_000)), {}) { false }
        assertEquals(LocalResult.Text("hello there"), result)
    }

    @Test
    fun silenceIsBlankText() {
        val result = LocalListening(FakeSession()).run(Mic(List(600) { quiet }), {}) { false }
        assertEquals(LocalResult.Text(""), result)
    }

    @Test
    fun cancelStopsWithoutDecoding() {
        val session = FakeSession(final = "never")
        var reads = 0
        val result = LocalListening(session).run(Mic(script(5_000, 0)), {}) { ++reads > 10 }
        assertEquals(LocalResult.Cancelled, result)
        assertTrue(!session.finished)
    }

    @Test
    fun aFailureKeepsWhatWasHeard() {
        val session = FakeSession(words = listOf("one", "one two"), failOnFeed = 2)
        val mic = Mic(script(2_000, 0))
        val result = LocalListening(session).run(mic, {}) { false }
        assertTrue(result is LocalResult.Failed)
        assertEquals("one two", (result as LocalResult.Failed).partial)
        assertTrue(mic.stopped)
    }

    @Test
    fun aFailureBeforeAnyWordsHasNothingToKeep() {
        val result = LocalListening(FakeSession(failOnFeed = 0)).run(Mic(script(2_000, 0)), {}) { false }
        assertEquals(null, (result as LocalResult.Failed).partial)
    }

    @Test
    fun theLimitEndsAStuckUtterance() {
        val session = FakeSession(final = "long")
        val mic = Mic(List(100_000) { speech })
        val result = LocalListening(session, limitBytes = 64_000).run(mic, {}) { false }
        assertEquals(LocalResult.Text("long"), result)
        assertTrue(mic.next * 512 <= 64_000 + 512)
    }

    @Test
    fun audioIsHandedOverInWholeSteps() {
        val session = FakeSession(final = "x")
        LocalListening(session, stepBytes = 5_120).run(Mic(script(1_000, 3_000)), {}) { false }
        // 5120 bytes is 10 reads of 512: far fewer feeds than reads.
        assertTrue(session.fed < session.bytes / 5_000 + 2)
    }
}

class EngineChoiceTest {
    @Test
    fun localOnlyWhenChosenDownloadedAndUsable() {
        assertEquals(DictationEngine.Local, engineFor(DictationEngine.Local, modelReady = true, localUsable = true))
        assertEquals(DictationEngine.Android, engineFor(DictationEngine.Local, modelReady = false, localUsable = true))
        assertEquals(DictationEngine.Android, engineFor(DictationEngine.Local, modelReady = true, localUsable = false))
        assertEquals(DictationEngine.Android, engineFor(DictationEngine.Android, modelReady = true, localUsable = true))
    }
}
