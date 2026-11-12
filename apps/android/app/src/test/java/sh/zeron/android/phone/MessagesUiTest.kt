package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MessagesUiTest {
    @Test
    fun theSendButtonAndHintNameTheTransport() {
        assertEquals("rcs", MessagesUi.transportOf("Send RCS message", "RCS message"))
        assertEquals("rcs", MessagesUi.transportOf("Send message", "Chat message"))
        assertEquals("sms", MessagesUi.transportOf("Send SMS message", "Text message"))
        assertEquals("sms", MessagesUi.transportOf(null, "Text message"))
        assertEquals("unknown", MessagesUi.transportOf("Send", null))
        assertEquals("unknown", MessagesUi.transportOf())
        // The conversation header says it too.
        assertEquals("sms", MessagesUi.transportOf("Send SMS", null, "Texting with (555) 888-0003 (SMS/MMS)"))
        assertEquals("rcs", MessagesUi.transportOf("Send", null, "Chatting with Emma"))
        assertTrue(MessagesUi.isComposeHeader("Texting with (555) 888-0003 (SMS/MMS)"))
        assertFalse(MessagesUi.isComposeHeader("Texting plans"))
    }

    @Test
    fun onlyTheComposeSendButtonCounts() {
        for (ok in listOf("Send", "send message", "Send RCS message", "Send SMS message", "Send SMS", "Send MMS message", "Send chat message ")) {
            assertTrue(ok, MessagesUi.isSendButton(ok))
        }
        for (no in listOf("Send attachment", "Send money", "Send a sticker", "", null, "Resend")) {
            assertFalse(no.toString(), MessagesUi.isSendButton(no))
        }
    }
}
