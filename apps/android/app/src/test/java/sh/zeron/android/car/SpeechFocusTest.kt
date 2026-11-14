package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeechFocusTest {
    private val log = ArrayList<String>()
    private var next = 0
    private val timers = ArrayList<Pair<Long, () -> Unit>>()
    private var now = 0L

    private val focus = SpeechFocus<Int>(
        acquire = { pause -> (++next).also { log += "request#$it${if (pause) "(pause)" else ""}" } },
        abandon = { log += "abandon#$it" },
        later = { delay, block ->
            val entry = (now + delay) to block
            timers += entry
            ({ timers.remove(entry); Unit })
        },
        holdOverMs = 2_500,
    )

    private fun advance(ms: Long) {
        now += ms
        timers.filter { it.first <= now }.forEach { timers.remove(it); it.second() }
    }

    @Test
    fun focusIsRequestedOnceAndKeptThroughGapsBetweenPieces() {
        focus.hold(false)
        // A slow network voice: seconds pass between pieces, nothing settles.
        advance(20_000)
        focus.hold(false)
        advance(20_000)
        assertEquals(listOf("request#1"), log)
        assertTrue(focus.isHeld)
    }

    @Test
    fun focusIsGivenBackOnceAfterTheLastPiece() {
        focus.hold(false)
        focus.settle()
        advance(2_000)
        assertTrue("held through the hold-over", focus.isHeld)
        advance(600)
        assertFalse(focus.isHeld)
        assertEquals(listOf("request#1", "abandon#1"), log)
    }

    @Test
    fun aReplyReadRightAfterAnotherKeepsTheSameFocus() {
        focus.hold(false)
        focus.settle()
        advance(1_000)
        focus.hold(false)
        focus.settle()
        advance(5_000)
        assertEquals(listOf("request#1", "abandon#1"), log)
    }

    @Test
    fun stoppingGivesFocusBackAtOnce() {
        focus.hold(false)
        focus.drop()
        focus.drop()
        assertEquals(listOf("request#1", "abandon#1"), log)
    }

    @Test
    fun switchingToPausingMusicTakesANewFocus() {
        focus.hold(false)
        focus.hold(true)
        assertEquals(listOf("request#1", "abandon#1", "request#2(pause)"), log)
    }

    @Test
    fun longTextIsCutAtSentencesAndLosesNothing() {
        val text = (1..40).joinToString(" ") { "This is sentence number $it." }
        val pieces = speechPieces(text, 240)
        assertTrue(pieces.size > 3)
        assertTrue(pieces.all { it.length <= 240 })
        assertTrue(pieces.dropLast(1).all { it.endsWith(".") })
        assertEquals(text, pieces.joinToString(" "))
    }

    @Test
    fun shortTextIsOnePieceAndAWallOfWordsStillSplits() {
        assertEquals(listOf("Done."), speechPieces("Done."))
        assertEquals(listOf("a b c"), speechPieces(" a b c "))
        val wall = "x".repeat(700)
        assertEquals(wall, speechPieces(wall, 240).joinToString(""))
    }
}
