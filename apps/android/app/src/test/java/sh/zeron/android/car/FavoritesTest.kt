package sh.zeron.android.car

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FavoritesTest {
    private fun agent(id: String, pinRank: Int? = null, state: AgentState = AgentState.Done) =
        CarAgent(id, id, state, "", "", false, Folder("project:p", "p"), 0, null, pinRank)

    @Test
    fun favoritesAreThePinnedChatsInPinOrder() {
        // The list is newest first; the sidebar's pin order wins for favorites.
        val agents = listOf(agent("new"), agent("b", pinRank = 1), agent("old"), agent("a", pinRank = 0))
        assertEquals(listOf("a", "b"), favorites(agents).map { it.id })
    }

    @Test
    fun noPinnedChatsMeansNoFavorites() {
        assertTrue(favorites(listOf(agent("x"), agent("y"))).isEmpty())
        assertEquals("None pinned yet", favoritesSummary(emptyList()))
    }

    @Test
    fun summaryNamesOneChatOrCountsAndFlagsWaiting() {
        assertEquals("a", favoritesSummary(listOf(agent("a", 0))))
        assertEquals("2 chats", favoritesSummary(listOf(agent("a", 0), agent("b", 1))))
        assertEquals("2 chats · 1 waiting", favoritesSummary(listOf(agent("a", 0), agent("b", 1, AgentState.NeedsInput))))
    }
}
