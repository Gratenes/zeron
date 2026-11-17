package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SendReportTest {
    @Test fun providerBoxes() {
        assertEquals("sent", SendReport.providerState(2))
        assertEquals("queued", SendReport.providerState(4))
        assertEquals("queued", SendReport.providerState(6))
        assertEquals("failed", SendReport.providerState(5))
    }

    @Test fun deliveryStatuses() {
        assertEquals("delivered", SendReport.deliveryOf(0))
        assertEquals("pending", SendReport.deliveryOf(32))
        assertEquals("pending", SendReport.deliveryOf(63))
        assertEquals("failed", SendReport.deliveryOf(64))
        assertEquals("failed", SendReport.deliveryOf(-1))
    }

    @Test fun onlyADeliveryReportConfirms() {
        assertTrue(SendReport.confirmed("delivered"))
        for (d in listOf("pending", "unknown", "failed", "sent")) assertFalse(SendReport.confirmed(d))
    }

    @Test fun anSmsWithoutAReportSaysItIsNotConfirmed() {
        val n = SendReport.note("sms", "unknown", null)
        assertTrue(n.contains("NOT confirmation"))
    }

    @Test fun failuresAreLoud() {
        assertTrue(SendReport.note("sms", "failed", null).startsWith("FAILED"))
        assertTrue(SendReport.note("messages", "unknown", "failed").startsWith("FAILED"))
        assertTrue(SendReport.note("messages", "unknown", "queued").startsWith("NOT SENT YET"))
    }
}
