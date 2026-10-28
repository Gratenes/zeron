package sh.zeron.android.car

import uniffi.zeron_core.ChatIndicator

/** An agent as the car sees it. */
enum class AgentState { Working, NeedsInput, Failed, Done, Idle }

fun ChatIndicator.agentState(): AgentState = when (this) {
    ChatIndicator.WORKING -> AgentState.Working
    ChatIndicator.AWAITING_INPUT -> AgentState.NeedsInput
    ChatIndicator.ERRORED -> AgentState.Failed
    ChatIndicator.COMPLETED -> AgentState.Done
    ChatIndicator.IDLE -> AgentState.Idle
}

/** Something an agent has to say: it finished, wants an answer, or failed. */
enum class ReportKind { Finished, NeedsInput, Failed }

/**
 * Turns successive workspace states into reports. Pure: the first look at a
 * chat only records it (nothing that happened before the car connected is
 * announced), and a chat must have been seen working to report finishing
 * or failing. A question is new whenever the chat wasn't already waiting.
 */
class AgentStates {
    private var known = HashMap<String, AgentState>()

    fun update(current: Map<String, AgentState>): List<Pair<String, ReportKind>> {
        val reports = mutableListOf<Pair<String, ReportKind>>()
        for ((id, now) in current) {
            val before = known[id] ?: continue
            if (before == now) continue
            when (now) {
                AgentState.NeedsInput -> reports += id to ReportKind.NeedsInput
                AgentState.Done, AgentState.Idle -> if (before == AgentState.Working) reports += id to ReportKind.Finished
                AgentState.Failed -> if (before == AgentState.Working || before == AgentState.NeedsInput) reports += id to ReportKind.Failed
                AgentState.Working -> Unit
            }
        }
        known = HashMap(current)
        return reports
    }

    fun reset() {
        known = HashMap()
    }
}

/** How long a turn has run: "40s", "12m", "1h 5m". */
fun elapsedLabel(sinceMs: Long, nowMs: Long = System.currentTimeMillis()): String {
    val seconds = ((nowMs - sinceMs) / 1000).coerceAtLeast(0)
    return when {
        seconds < 60 -> "${seconds}s"
        seconds < 3600 -> "${seconds / 60}m"
        else -> "${seconds / 3600}h ${seconds % 3600 / 60}m"
    }
}
