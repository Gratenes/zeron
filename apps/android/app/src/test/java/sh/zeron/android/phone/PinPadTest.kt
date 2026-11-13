package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PinPadTest {
    private val sys = "com.android.systemui"

    /** The nodes Nothing OS's bouncer showed over adb: ids keyN, digit as content description. */
    @Test fun nothingOsKeysAreFoundById() {
        for (d in '0'..'9') assertEquals(d, PinPad.digit(PadNode("$sys:id/key$d", "", "$d", sys, true), sys))
    }

    @Test fun keysCountEvenIfReportedInvisible() =
        assertEquals('7', PinPad.digit(PadNode("$sys:id/key7", "", "7", sys, false), sys))

    @Test fun otherIdsAreNotKeys() {
        assertNull(PinPad.digit(PadNode("$sys:id/keyguard_pin_view", "", "", sys, true), sys))
        assertNull(PinPad.digit(PadNode("$sys:id/key_enter", "", "", sys, true), sys))
        assertNull(PinPad.digit(PadNode("$sys:id/pinEntry", "", "PIN area", sys, true), sys))
    }

    @Test fun aBareDigitLabelNeedsToBeVisibleAndOfThePadsPackage() {
        assertEquals('5', PinPad.digit(PadNode(null, "5", null, sys, true), sys))
        assertNull(PinPad.digit(PadNode(null, "5", null, sys, false), sys))
        assertNull(PinPad.digit(PadNode(null, "5", null, "com.other", true), sys))
    }

    @Test fun enterIsFoundByIdOrLabel() {
        assertTrue(PinPad.isEnter(PadNode("$sys:id/key_enter", "", "", sys, true)))
        assertTrue(PinPad.isEnter(PadNode(null, "", "Enter", sys, true)))
        assertTrue(!PinPad.isEnter(PadNode(null, "", "Emergency", sys, true)))
    }
}
