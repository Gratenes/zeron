package sh.zeron.android.car

/**
 * Agent replies made fit to be read aloud: markdown, code, links and tables
 * dropped, cut at a sentence boundary. Pure; the car's text-to-speech and
 * screens both use it.
 */
object Spoken {
    private val fence = Regex("(?s)```.*?(```|$)")
    private val image = Regex("!\\[([^\\]]*)]\\([^)]*\\)")
    private val link = Regex("\\[([^\\]]+)]\\([^)]*\\)")
    private val url = Regex("\\bhttps?://\\S+")
    private val inlineCode = Regex("`([^`]*)`")
    private val emphasis = Regex("(\\*\\*|__|\\*|_|~~)(?=\\S)(.+?)(?<=\\S)\\1")
    private val heading = Regex("^#{1,6}\\s+")
    private val bullet = Regex("^(\\s*)([-*+]|\\d+[.)])\\s+")
    private val quote = Regex("^>\\s?")
    private val tableRule = Regex("^\\s*\\|?\\s*:?-{3,}")
    private val space = Regex("\\s+")
    private val sentenceEnd = Regex("(?<=[.!?])\\s+")

    /** Plain prose: paragraphs separated by blank lines, list items as sentences. */
    fun clean(markdown: String): String {
        val text = fence.replace(markdown, "\n\n")
        val paragraphs = mutableListOf<String>()
        val current = StringBuilder()
        fun flush() {
            val p = space.replace(current.toString(), " ").trim()
            if (p.isNotEmpty()) paragraphs.add(p)
            current.clear()
        }
        for (raw in text.lines()) {
            var line = raw.trimEnd()
            if (line.isBlank()) {
                flush()
                continue
            }
            // Tables and separators read as noise.
            if (line.trimStart().startsWith("|") || tableRule.containsMatchIn(line) || line.trim().matches(Regex("[-*_]{3,}"))) continue
            line = heading.replace(line, "")
            line = quote.replace(line, "")
            val isItem = bullet.containsMatchIn(line)
            line = bullet.replace(line, "")
            line = image.replace(line, "$1")
            line = link.replace(line, "$1")
            line = url.replace(line, "")
            line = inlineCode.replace(line, "$1")
            line = emphasis.replace(line, "$2")
            line = line.trim()
            if (line.isEmpty()) continue
            if (isItem && current.isNotEmpty() && current.last() !in ".!?:;") current.append('.')
            if (isItem && line.last() !in ".!?:;") line += "."
            if (current.isNotEmpty()) current.append(' ')
            current.append(line)
        }
        flush()
        return paragraphs.joinToString("\n\n")
    }

    /**
     * The opening of [markdown] as speech, whole sentences only, at most
     * [maxChars] (one sentence is cut with an ellipsis when it alone is longer).
     */
    fun summary(markdown: String, maxChars: Int = 280): String {
        val prose = clean(markdown)
        if (prose.isEmpty()) return ""
        val first = prose.substringBefore("\n\n")
        val sentences = sentenceEnd.split(prose.replace("\n\n", " "))
        val out = StringBuilder()
        for (s in sentences) {
            val next = if (out.isEmpty()) s else "$out $s"
            if (next.length > maxChars) break
            out.clear().append(next)
            // A whole first paragraph is a natural stopping point.
            if (out.length >= first.length) break
        }
        if (out.isEmpty()) {
            val cut = sentences.first().take(maxChars)
            return cut.substringBeforeLast(' ', cut).trimEnd(',', ';', ':') + "…"
        }
        return out.toString()
    }

    /**
     * The newest whole sentences of [markdown] as prose, at most [maxChars]
     * (a running agent's latest progress, while its message is still growing).
     */
    fun latest(markdown: String, maxChars: Int = 200): String {
        val prose = clean(markdown).replace("\n\n", " ")
        if (prose.length <= maxChars) return prose
        val sentences = sentenceEnd.split(prose)
        val out = ArrayDeque<String>()
        var length = 0
        for (s in sentences.asReversed()) {
            if (out.isNotEmpty() && length + s.length + 1 > maxChars) break
            out.addFirst(s)
            length += s.length + 1
        }
        val text = out.joinToString(" ")
        return if (text.length <= maxChars) "…$text" else "…" + text.takeLast(maxChars).substringAfter(' ')
    }

    private val ordinals = listOf(
        listOf("first", "one", "1", "option one", "option 1"),
        listOf("second", "two", "2", "option two", "option 2"),
        listOf("third", "three", "3", "option three", "option 3"),
        listOf("fourth", "four", "4", "option four", "option 4"),
    )

    private fun words(s: String) = s.lowercase().replace(Regex("[^a-z0-9 ]"), " ").let { space.replace(it, " ").trim() }

    /**
     * The option a spoken answer picks ("the second one", "yes", most of an
     * option's words), or null when it picks none (then it's free text).
     */
    fun matchOption(spoken: String, options: List<String>): String? {
        if (options.isEmpty()) return null
        val said = words(spoken).removePrefix("the ").removeSuffix(" one").removeSuffix(" option").trim()
        options.firstOrNull { words(it) == said }?.let { return it }
        ordinals.forEachIndexed { i, names -> if (i < options.size && said in names) return options[i] }
        // An option named inside the answer ("go with the staging deploy").
        val named = options.filter { words(it).isNotEmpty() && " $said ".contains(" ${words(it)} ") }
        if (named.size == 1) return named[0]
        val saidWords = said.split(' ').toSet()
        // Words only one option has ("staging" for "Deploy to staging").
        val key = saidWords.filter { it.length > 2 }
        if (key.isNotEmpty()) {
            val having = options.filter { o -> words(o).split(' ').toSet().containsAll(key) }
            if (having.size == 1) return having[0]
        }
        // Most of one option's words, and clearly more than any other's.
        val scored = options.map { o ->
            val w = words(o).split(' ').filter { it.length > 2 }
            o to if (w.isEmpty()) 0.0 else w.count { it in saidWords }.toDouble() / w.size
        }.sortedByDescending { it.second }
        val best = scored.first()
        val runnerUp = scored.getOrNull(1)?.second ?: 0.0
        return if (best.second >= 0.6 && best.second > runnerUp) best.first else null
    }

    private val all = setOf("all", "all of them", "all of those", "all of the above", "everything", "every one")
    private val joiners = Regex(",|;|&|\\b(and|plus|or)\\b", RegexOption.IGNORE_CASE)

    /**
     * The options a spoken answer picks when several may be ("staging and
     * production", "the first and the third", "all of them"), in the
     * options' order; empty when it picks none.
     */
    fun matchOptions(spoken: String, options: List<String>): List<String> {
        if (options.isEmpty()) return emptyList()
        val said = words(spoken).removePrefix("the ")
        if (said in all || said.startsWith("both") && options.size == 2) return options
        val parts = spoken.split(joiners).map { it.trim() }.filter { it.isNotEmpty() }
        val each = parts.mapNotNull { matchOption(it, options) }.toSet()
        if (each.size >= 2) return options.filter { it in each }
        // One option whose own name has an "and" in it ("Search and replace").
        matchOption(spoken, options)?.let { return listOf(it) }
        return options.filter { it in each }
    }
}
