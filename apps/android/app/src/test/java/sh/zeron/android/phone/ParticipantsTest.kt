package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Test

class ParticipantsTest {
    private val emma = "+14702728916"
    private val me = "+16785550100"

    @Test fun receivedRcsListsSenderAndSelf() =
        assertEquals(listOf(emma), Participants.others(true, emma, listOf(me), listOf(me)))

    @Test fun receivedRcsWithUnknownSelfIsStillOnePerson() =
        assertEquals(listOf(emma), Participants.others(true, emma, listOf(me), emptyList()))

    @Test fun sentRcsListsRecipientAndSelf() =
        assertEquals(listOf(emma), Participants.others(false, me, listOf(emma, me), listOf(me)))

    @Test fun numbersWrittenDifferentlyMatch() =
        assertEquals(listOf(emma), Participants.others(false, me, listOf(emma, "(678) 555-0100"), listOf(me)))

    @Test fun realGroupStaysAGroup() =
        assertEquals(listOf(emma, "+14045550123"), Participants.others(true, emma, listOf(me, "+14045550123"), listOf(me)))
}
