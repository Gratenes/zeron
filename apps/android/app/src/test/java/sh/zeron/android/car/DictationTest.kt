package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DictationTest {
    private val speech = ByteArray(1600) { i -> if (i % 2 == 0) 0 else 0x20 } // 50 ms, loud
    private val quiet = ByteArray(1600)

    /** A stream opens on a moment of room noise, which sets the floor. */
    private fun Endpointer.start() = feedMs(quiet, 200)

    private fun Endpointer.feedMs(buffer: ByteArray, ms: Int): Boolean {
        var ended = false
        repeat(ms / 50) { if (feed(buffer, buffer.size)) ended = true }
        return ended
    }

    @Test
    fun aPauseToThinkMidSentenceDoesNotEndTheReply() {
        val e = Endpointer().also { it.start() }
        assertFalse(e.feedMs(speech, 1_000))
        // Two seconds of quiet: a driver choosing words, not finishing.
        assertFalse(e.feedMs(quiet, 1_900))
        assertFalse(e.feedMs(speech, 1_000))
    }

    @Test
    fun aLongQuietAfterSpeechEndsIt() {
        val e = Endpointer().also { it.start() }
        e.feedMs(speech, 1_000)
        assertTrue(e.feedMs(quiet, 2_500))
    }

    @Test
    fun silenceThroughoutGivesUp() {
        assertTrue(Endpointer().feedMs(quiet, 9_000))
    }

    @Test
    fun theFinalTextWinsUnlessItWasCutShort() {
        assertEquals("send the patch", heardText("send the patch", "send the"))
        assertEquals("send the patch to staging", heardText("send the patch", "send the patch to staging"))
        assertEquals("only partial", heardText(null, "only partial"))
        assertEquals("only partial", heardText("  ", "only partial"))
        assertNull(heardText(null, null))
    }
}
