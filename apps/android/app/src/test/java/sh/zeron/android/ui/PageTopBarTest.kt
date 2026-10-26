package sh.zeron.android.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class PageTopBarTest {
    @Test fun nothingIsDimmedAtRest() {
        assertEquals(0f, topEdgeFadeAlpha(0f), 0f)
        // Overscroll at the top.
        assertEquals(0f, topEdgeFadeAlpha(-30f), 0f)
    }

    @Test fun fadeComesInOverItsOwnHeight() {
        assertEquals(0.5f, topEdgeFadeAlpha(TopEdgeFadeHeight.value / 2), 0.001f)
        assertEquals(1f, topEdgeFadeAlpha(TopEdgeFadeHeight.value), 0f)
        assertEquals(1f, topEdgeFadeAlpha(5000f), 0f)
    }
}
