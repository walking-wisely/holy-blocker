package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BlocklistProvisioningTest {

    private val v1 = byteArrayOf(1, 2, 3)
    private val v2 = byteArrayOf(1, 2, 4)

    private fun slot(manifest: ByteArray, length: Long) = SlotFingerprint(manifest, length)

    @Test
    fun `a bundled slot is installed when nothing is installed`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(v1, 10), installed = null))
    }

    @Test
    fun `a bundled slot replaces an installed one whose manifest differs`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(v2, 10), installed = slot(v1, 10)))
    }

    @Test
    fun `a truncated installed artifact under an unchanged manifest is reinstalled`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(v1, 10), installed = slot(v1, 4)))
    }

    @Test
    fun `an identical slot is left alone`() {
        assertFalse(BlocklistProvisioning.needsInstall(bundled = slot(v1, 10), installed = slot(v1.copyOf(), 10)))
    }

    @Test
    fun `nothing bundled never overwrites what is installed`() {
        assertFalse(BlocklistProvisioning.needsInstall(bundled = null, installed = slot(v1, 10)))
        assertFalse(BlocklistProvisioning.needsInstall(bundled = null, installed = null))
    }

    @Test
    fun `a key file name is its key id`() {
        assertEquals("release-1", BlocklistProvisioning.keyId("release-1.pub"))
    }

    @Test
    fun `a file that is not a key is not a key id`() {
        assertNull(BlocklistProvisioning.keyId("release-1.txt"))
        assertNull(BlocklistProvisioning.keyId(".pub"))
    }

    @Test
    fun `every failure maps to its own recorded event`() {
        val events = BlocklistFailure.entries.map { BlocklistProvisioning.eventFor(it) }
        assertEquals(events.size, events.toSet().size)
        assertEquals(TamperEvent.BLOCKLIST_MISSING, BlocklistProvisioning.eventFor(BlocklistFailure.MISSING))
        assertEquals(TamperEvent.BLOCKLIST_REJECTED, BlocklistProvisioning.eventFor(BlocklistFailure.REJECTED))
    }
}
