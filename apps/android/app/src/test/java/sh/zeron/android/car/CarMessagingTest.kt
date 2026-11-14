package sh.zeron.android.car

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CarMessagingTest {
    private fun report(kind: ReportKind, text: String = "", spoken: String = "", title: String = "Fix login") =
        Report("chat1", title, kind, text, spoken, orchestrator = false)

    @Test
    fun aFinishedReportReadsTheWholeReplyAsProse() {
        val reply = (1..30).joinToString(" ") { "Sentence number $it." }
        val body = CarMessaging.body(report(ReportKind.Finished, text = reply))
        // Assistant reads it all, not the 120-character gist.
        assertTrue(body.length > 400)
        assertTrue(body.startsWith("Sentence number 1."))
    }

    @Test
    fun aFinishedReportWithNoTextStillSaysDone() {
        assertEquals("Done.", CarMessaging.body(report(ReportKind.Finished)))
    }

    @Test
    fun aQuestionReadsTheQuestionWithoutTheNameInFront() {
        val body = CarMessaging.body(report(ReportKind.NeedsInput, spoken = "Fix login needs you. Which branch should I use?"))
        assertEquals("Which branch should I use?", body)
    }

    @Test
    fun aFailureSaysSo() {
        assertEquals("Ran into an error.", CarMessaging.body(report(ReportKind.Failed)))
    }

    @Test
    fun dictationIsTrimmedAndEmptyIsNothing() {
        assertEquals("yes go ahead", CarMessaging.dictated("  yes go ahead \n"))
        assertNull(CarMessaging.dictated("   "))
        assertNull(CarMessaging.dictated(null))
    }

    @Test
    fun aVoiceReplyGoesToItsChat() = runBlocking {
        var sent: Pair<String, String>? = null
        val result = CarMessaging.reply("chat1", " ship it ") { id, text ->
            sent = id to text
            CarAgents.Outcome.Sent(id)
        }
        assertEquals(CarMessaging.Result.Sent("chat1"), result)
        assertEquals("chat1" to "ship it", sent)
    }

    @Test
    fun nothingDictatedOrNoChatSendsNothing() = runBlocking {
        var calls = 0
        val send: suspend (String, String) -> CarAgents.Outcome = { id, _ -> calls++; CarAgents.Outcome.Sent(id) }
        assertEquals(CarMessaging.Result.Nothing, CarMessaging.reply("chat1", "  ", send))
        assertEquals(CarMessaging.Result.Nothing, CarMessaging.reply(null, "hello", send))
        assertEquals(CarMessaging.Result.Nothing, CarMessaging.reply("", "hello", send))
        assertEquals(0, calls)
    }

    @Test
    fun aReplyThatFailsSaysWhy() = runBlocking {
        assertEquals(
            CarMessaging.Result.Failed("None of your computers is online."),
            CarMessaging.reply("c", "hi") { _, _ -> CarAgents.Outcome.NoHost },
        )
        assertEquals(
            CarMessaging.Result.Failed("Sign in to Zeron on your phone first."),
            CarMessaging.reply("c", "hi") { _, _ -> CarAgents.Outcome.SignedOut },
        )
        assertEquals(
            CarMessaging.Result.Failed("Couldn't send that."),
            CarMessaging.reply("c", "hi") { _, _ -> CarAgents.Outcome.Failed("Couldn't send that.") },
        )
    }
}
