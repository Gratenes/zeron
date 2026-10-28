package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SpokenTest {
    @Test
    fun dropsMarkdownCodeAndLinks() {
        val reply = """
            ## Done

            I fixed the **login redirect** in `auth.rs` — see [the PR](https://github.com/x/y/pull/1).

            ```rust
            fn main() {}
            ```

            - Added a test
            - Updated docs

            | a | b |
            |---|---|
            | 1 | 2 |
        """.trimIndent()
        assertEquals(
            "Done\n\nI fixed the login redirect in auth.rs — see the PR.\n\nAdded a test. Updated docs.",
            Spoken.clean(reply),
        )
    }

    @Test
    fun summaryStopsAtTheFirstParagraphOrTheLimit() {
        assertEquals("All tests pass.", Spoken.summary("All tests pass.\n\nI also cleaned up the imports."))
        val long = "First sentence here. Second sentence is a bit longer. Third one too."
        assertEquals("First sentence here. Second sentence is a bit longer.", Spoken.summary(long, 60))
        assertEquals("Supercalifragilistic…", Spoken.summary("Supercalifragilistic expialidocious", 25))
    }

    @Test
    fun matchesSpokenOptions() {
        val options = listOf("Deploy to staging", "Deploy to production", "Cancel")
        assertEquals("Cancel", Spoken.matchOption("cancel", options))
        assertEquals("Deploy to production", Spoken.matchOption("the second one", options))
        assertEquals("Deploy to staging", Spoken.matchOption("go with deploy to staging please", options))
        assertEquals("Deploy to production", Spoken.matchOption("production deploy", options))
        assertEquals("Deploy to staging", Spoken.matchOption("staging", options))
        assertNull(Spoken.matchOption("deploy", options))
        assertNull(Spoken.matchOption("wait until tomorrow", options))
    }
}

class AgentStatesTest {
    @Test
    fun firstLookOnlyRecords() {
        val states = AgentStates()
        assertEquals(emptyList<Any>(), states.update(mapOf("a" to AgentState.Done, "b" to AgentState.NeedsInput)))
    }

    @Test
    fun reportsTransitions() {
        val states = AgentStates()
        states.update(mapOf("a" to AgentState.Working, "b" to AgentState.Working, "c" to AgentState.Idle))
        val reports = states.update(mapOf("a" to AgentState.Done, "b" to AgentState.NeedsInput, "c" to AgentState.Done))
        assertEquals(listOf("a" to ReportKind.Finished, "b" to ReportKind.NeedsInput), reports)
        assertEquals(listOf("b" to ReportKind.Failed), states.update(mapOf("a" to AgentState.Idle, "b" to AgentState.Failed)))
        // A chat that disappeared and came back is new again.
        assertEquals(emptyList<Any>(), states.update(mapOf("a" to AgentState.Working)))
    }
}

class EndpointerTest {
    /** [ms] of a 16 kHz tone at [amplitude], as 512-byte buffers. */
    private fun audio(ms: Int, amplitude: Int): List<ByteArray> = (0 until ms * 16 / 256).map { b ->
        ByteArray(512).also { buf ->
            for (i in 0 until 256) {
                val v = (amplitude * kotlin.math.sin((b * 256 + i) * 2 * Math.PI * 300 / 16_000)).toInt()
                buf[2 * i] = v.toByte()
                buf[2 * i + 1] = (v shr 8).toByte()
            }
        }
    }

    private fun Endpointer.endsAfter(buffers: List<ByteArray>): Int? =
        buffers.indexOfFirst { feed(it, it.size) }.takeIf { it >= 0 }

    @Test
    fun endsAfterSpeechThenSilence() {
        val e = Endpointer(silenceMs = 1_200)
        assertNull(e.endsAfter(audio(500, 40) + audio(1_500, 8_000) + audio(1_000, 40)))
        assertEquals(true, e.spoke)
        // 1.2 s of quiet after the 1 s already fed: done about 200 ms in.
        val at = e.endsAfter(audio(1_000, 40))!!
        assertEquals(true, at in 10..14)
    }

    @Test
    fun givesUpWithoutSpeech() {
        val e = Endpointer(waitMs = 3_000)
        assertNull(e.endsAfter(audio(2_900, 300)))
        assertEquals(false, e.spoke)
        assertEquals(true, e.endsAfter(audio(500, 300)) != null)
    }
}

class ElapsedLabelTest {
    @Test
    fun readsLikeAClock() {
        assertEquals("40s", elapsedLabel(0, 40_000))
        assertEquals("12m", elapsedLabel(0, 12 * 60_000 + 30_000))
        assertEquals("1h 5m", elapsedLabel(0, 65 * 60_000))
        assertEquals("0s", elapsedLabel(10_000, 0))
    }
}

class LatestTest {
    @Test
    fun keepsTheNewestSentences() {
        assertEquals("I'll look into this.", Spoken.latest("I'll look into this."))
        val text = "Looking at the auth flow. Found the redirect bug in **login.rs**. Writing a fix now."
        assertEquals("…Found the redirect bug in login.rs. Writing a fix now.", Spoken.latest(text, 60))
        assertEquals("…a fix now.", Spoken.latest("Writing a fix now.", 12))
    }
}

class ReportTextTest {
    @Test
    fun anIntroDoesNotEndTheSummary() {
        val reply = "Here's what I found:\n\n- Three calls are overdue\n- Acme asked for a quote"
        assertEquals("Here's what I found: Three calls are overdue. Acme asked for a quote.", Spoken.summary(reply))
        // A real first paragraph still ends a spoken report…
        val full = "Fixed the login redirect and added a test for it.\n\nAlso tidied the imports."
        assertEquals("Fixed the login redirect and added a test for it.", Spoken.summary(full))
        // …but screens and Read aloud take the rest too.
        assertEquals("Fixed the login redirect and added a test for it. Also tidied the imports.", Spoken.summary(full, 600, wholeText = true))
    }

    @Test
    fun taskBoxesAreDropped() {
        assertEquals("Plan: Create the archive. Move old files.", Spoken.clean("Plan:\n- [x] Create the archive\n- [ ] Move old files"))
    }

    @Test
    fun folderCodes() {
        assertEquals("ZE", folderCode(Folder("project:1", "zeron · MacBook Pro")))
        assertEquals("EP", folderCode(Folder("project:2", "embedez-platform")))
        assertEquals("CS", folderCode(Folder("project:3", "CGI Salesforce")))
        assertEquals("~", folderCode(Folder("home:dev", "No project · MacBook Pro")))
    }
}
