package sh.zeron.android.car

import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.UserInputQuestion

/**
 * An agent's questions answered one at a time, by voice or touch, and sent
 * together once each has an answer (a request is answered once, whole).
 * Pure: the car screens and talk turns drive it.
 */
class QuestionFlow(val input: InputRequest) {
    sealed interface Step {
        /** Ask [question] next; [prompt] is what's read aloud. */
        data class Ask(val index: Int, val question: UserInputQuestion, val prompt: String) : Step

        /** Every question has its answer: send these. */
        data class Submit(val answers: List<UserInputAnswer>) : Step
    }

    val questions: List<UserInputQuestion> get() = input.questions
    private val answers = ArrayList<UserInputAnswer>()

    /** The question being asked (0-based); [questions].size once all are answered. */
    val index: Int get() = answers.size

    /** "1 of 2" when there's more than one question, else empty. */
    val progress: String get() = if (questions.size > 1 && index < questions.size) "${index + 1} of ${questions.size}" else ""

    fun step(): Step = questions.getOrNull(index)?.let { Step.Ask(index, it, prompt(index)) } ?: Step.Submit(answers.toList())

    /** The current question answered with [labels] (picked options or free text). */
    fun answer(labels: List<String>): Step {
        val q = questions.getOrNull(index) ?: return step()
        answers += UserInputAnswer(q.id, labels)
        return step()
    }

    /** The current question answered by a spoken [text]. */
    fun hear(text: String): Step = questions.getOrNull(index)?.let { answer(labels(it, text)) } ?: step()

    /** What's sent, read back: "Sending: Local device; Claude Code and Codex." */
    fun confirmation(answers: List<UserInputAnswer>): String =
        "Sending: " + answers.joinToString("; ") { a -> spokenList(a.labels.map { Spoken.summary(it, 80) }) } + "."

    private fun prompt(i: Int): String {
        val q = questions[i]
        val lead = if (questions.size > 1) "Question ${i + 1} of ${questions.size}. " else ""
        val options = q.options.take(MAX_SPOKEN_OPTIONS).map(Spoken::clean)
        val choices = when {
            options.isEmpty() -> ""
            q.multiSelect && options.size > 1 -> " Pick any of: " + spokenList(options, "or") + "."
            options.size == 1 -> " Option: ${options[0]}."
            else -> " Options: " + spokenList(options, "or") + "."
        }
        return lead + Spoken.clean(q.question) + choices
    }

    companion object {
        private const val MAX_SPOKEN_OPTIONS = 4

        /** A spoken answer as labels: the options it names, else the words themselves. */
        fun labels(q: UserInputQuestion, text: String): List<String> {
            val words = text.trim()
            if (q.options.isEmpty()) return listOf(words)
            val picked = if (q.multiSelect) Spoken.matchOptions(words, q.options) else listOfNotNull(Spoken.matchOption(words, q.options))
            return picked.ifEmpty { listOf(words) }
        }

        private fun spokenList(items: List<String>, last: String = "and"): String = when (items.size) {
            0 -> ""
            1 -> items[0]
            else -> items.dropLast(1).joinToString(", ") + (if (items.size > 2) "," else "") + " $last " + items.last()
        }
    }
}
