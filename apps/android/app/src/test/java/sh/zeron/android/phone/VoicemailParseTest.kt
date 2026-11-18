package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VoicemailParseTest {
    private val gv = "com.google.android.apps.googlevoice"
    private val phone = "com.google.android.dialer"

    @Test fun googleVoiceTitleWithNumberAndTranscriptInBigText() {
        val v = VoicemailParse.parse(gv, "New voicemail from (404) 555-0123", "Hi this is Dan calling about the house on Old Fountain…", "Hi this is Dan calling about the house on Old Fountain, please call me back.", emptyList(), null)
        assertNotNull(v)
        assertEquals("(404) 555-0123", v!!.caller)
        assertEquals("Hi this is Dan calling about the house on Old Fountain, please call me back.", v.transcript)
        assertEquals("Google Voice", v.source)
    }

    @Test fun contactNameCaller() {
        val v = VoicemailParse.parse(gv, "Voicemail from Josefina Lora", "Call me back", null, emptyList(), null)
        assertEquals("Josefina Lora", v!!.caller)
        assertEquals("Call me back", v.transcript)
    }

    @Test fun messagingStyleTranscript() {
        val v = VoicemailParse.parse(phone, "New voicemail", "", null, listOf("From (470) 272-8916", "Hey it's me, call when you can"), null)
        assertEquals("(470) 272-8916", v!!.caller)
        assertEquals("Hey it's me, call when you can", v.transcript)
        assertEquals("Google Phone", v.source)
    }

    @Test fun aPlaceholderIsNotATranscript() {
        val v = VoicemailParse.parse(gv, "New voicemail from +14045550123", "Transcribing…", null, emptyList(), null)
        assertEquals("+14045550123", v!!.caller)
        assertEquals("", v.transcript)
    }

    @Test fun otherNotificationsAndOtherAppsAreIgnored() {
        assertFalse(VoicemailParse.looksLikeVoicemail(gv, "texts", "New message from Dan", "hello", null))
        assertFalse(VoicemailParse.looksLikeVoicemail("com.whatsapp", null, "New voicemail from Dan", "x", null))
        assertNull(VoicemailParse.parse(gv, "Missed call from Dan", "", null, emptyList(), null))
        assertTrue(VoicemailParse.looksLikeVoicemail(gv, "voicemail_channel", "Dan", "hi", null))
    }
}
