package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReactionsTest {
    /** The row the real phone had: thread 1206, m12152. */
    @Test fun googleMessagesReactionRow() {
        val r = Reactions.parse(" ​👍​ to “ Hey Andrea, yes we're accepting offers. Send it over whenever your buyers are ready. ” ")
        assertNotNull(r)
        assertEquals("👍", r!!.emoji)
        assertEquals("Hey Andrea, yes we're accepting offers. Send it over whenever your buyers are ready.", r.to)
    }

    @Test fun tapbacksFromAnIPhone() {
        val r = Reactions.parse("Loved “See you at 5”")
        assertEquals("❤️", r!!.emoji)
        assertEquals("See you at 5", r.to)
        assertEquals("😂", Reactions.parse("Laughed at “ha”")!!.emoji)
    }

    @Test fun ordinaryTextIsNotAReaction() {
        assertNull(Reactions.parse("Thanks, I'll be there at 3"))
        assertNull(Reactions.parse("Going to “the store”"))
        assertNull(Reactions.parse("ok to “go”"))
    }

    @Test fun matchesASentReactionToATarget() {
        val row = "😂​ to “Lockbox code is 1111. Let me know how it goes!”"
        assertTrue(Reactions.matches(row, "😂", "Lockbox code is 1111. Let me know how it goes!"))
        assertFalse(Reactions.matches(row, "👍", "Lockbox code is 1111"))
        assertFalse(Reactions.matches(row, "😂", "something else entirely"))
        assertTrue(Reactions.matches("❤ to “hi”", "❤️", "hi"))
    }
}
