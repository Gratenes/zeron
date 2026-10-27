package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.UserInputQuestion

class QuestionFlowTest {
    private fun question(id: String, text: String, options: List<String> = emptyList(), multi: Boolean = false) =
        UserInputQuestion(id, "", text, options, multi, null, false)

    private val source = question("q1", "Which device should serve the catalogs?", listOf("Space's device (Recommended)", "Local device", "Union of both"))
    private val harnesses = question("q2", "Which harnesses should phones offer?", listOf("Claude Code", "Codex", "OpenCode", "Grok"), multi = true)
    private val note = question("q3", "Anything else?")

    private fun flow(vararg questions: UserInputQuestion) = QuestionFlow(InputRequest("e1", "req-1", questions.toList()))

    @Test
    fun asksOneQuestionAtATimeThenSubmitsEveryAnswer() {
        val flow = flow(source, harnesses)
        val first = flow.step() as QuestionFlow.Step.Ask
        assertEquals(0, first.index)
        assertEquals("1 of 2", flow.progress)
        assertEquals(
            "Question 1 of 2. Which device should serve the catalogs? Options: Space's device (Recommended), Local device, or Union of both.",
            first.prompt,
        )
        val second = flow.hear("local device") as QuestionFlow.Step.Ask
        assertEquals("q2", second.question.id)
        assertEquals(
            "Question 2 of 2. Which harnesses should phones offer? Pick any of: Claude Code, Codex, OpenCode, or Grok.",
            second.prompt,
        )
        val done = flow.hear("Claude Code and Codex") as QuestionFlow.Step.Submit
        assertEquals(
            listOf(UserInputAnswer("q1", listOf("Local device")), UserInputAnswer("q2", listOf("Claude Code", "Codex"))),
            done.answers,
        )
        assertEquals("Sending: Local device; Claude Code and Codex.", flow.confirmation(done.answers))
    }

    @Test
    fun unmatchedOrOptionlessAnswersAreFreeText() {
        val flow = flow(source, note)
        flow.hear("  whichever is faster ")
        val done = flow.hear("ship it tonight") as QuestionFlow.Step.Submit
        assertEquals(
            listOf(UserInputAnswer("q1", listOf("whichever is faster")), UserInputAnswer("q3", listOf("ship it tonight"))),
            done.answers,
        )
    }

    @Test
    fun tappedAnswersAdvanceTheSameWay() {
        val flow = flow(source, harnesses)
        assertEquals(1, (flow.answer(listOf("Union of both")) as QuestionFlow.Step.Ask).index)
        val done = flow.answer(listOf("Grok")) as QuestionFlow.Step.Submit
        assertEquals(listOf("Union of both"), done.answers[0].labels)
        assertEquals("", flow.progress)
    }

    @Test
    fun aSingleQuestionHasNoCount() {
        val flow = flow(question("q", "Deploy now?", listOf("Yes")))
        assertEquals("Deploy now? Option: Yes.", (flow.step() as QuestionFlow.Step.Ask).prompt)
        assertEquals(listOf(UserInputAnswer("q", listOf("Yes"))), (flow.hear("yes") as QuestionFlow.Step.Submit).answers)
    }

    @Test
    fun singleSelectPicksOneOption() {
        assertEquals(listOf("Space's device (Recommended)"), QuestionFlow.labels(source, "the first one"))
        // Two options named for a single choice: neither wins, so it's sent as said.
        assertEquals(listOf("local device and union of both"), QuestionFlow.labels(source, "local device and union of both"))
    }
}

class MatchOptionsTest {
    private val envs = listOf("Deploy to staging", "Deploy to production", "Deploy to preview")

    @Test
    fun picksSeveralOptions() {
        assertEquals(listOf("Deploy to staging", "Deploy to production"), Spoken.matchOptions("staging and production", envs))
        assertEquals(listOf("Deploy to staging", "Deploy to preview"), Spoken.matchOptions("the first and the third", envs))
        assertEquals(listOf("Deploy to staging", "Deploy to production"), Spoken.matchOptions("production, staging", envs))
        assertEquals(envs, Spoken.matchOptions("all of them", envs))
        assertEquals(listOf("Deploy to production"), Spoken.matchOptions("just production deploy", envs))
        assertEquals(emptyList<String>(), Spoken.matchOptions("none for now", envs))
    }

    @Test
    fun keepsOptionsWithAndInTheirName() {
        val options = listOf("Search and replace", "Rename symbol")
        assertEquals(listOf("Search and replace"), Spoken.matchOptions("search and replace", options))
        assertEquals(options, Spoken.matchOptions("both", options))
    }
}
