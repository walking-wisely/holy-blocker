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

    private fun manifest(version: Long) =
        ByteArray(8) { (version shr (8 * it)).toByte() } + byteArrayOf(9, 9)

    @Test
    fun `the manifest version is its first eight bytes, little endian`() {
        assertEquals(1L, BlocklistProvisioning.manifestVersion(manifest(1)))
        assertEquals(0x0102L, BlocklistProvisioning.manifestVersion(manifest(0x0102)))
    }

    @Test
    fun `a manifest too short to carry a version has none`() {
        assertNull(BlocklistProvisioning.manifestVersion(ByteArray(7)))
    }

    @Test
    fun `a bundled slot older than the installed one is not installed`() {
        assertFalse(BlocklistProvisioning.needsInstall(bundled = slot(manifest(3), 10), installed = slot(manifest(4), 10)))
    }

    @Test
    fun `a bundled slot newer than the installed one is installed`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(manifest(5), 10), installed = slot(manifest(4), 10)))
    }

    @Test
    fun `an installed manifest with no readable version is replaced`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(manifest(3), 10), installed = slot(byteArrayOf(1), 10)))
    }

    @Test
    fun `the same version with a different artifact length is reinstalled`() {
        assertTrue(BlocklistProvisioning.needsInstall(bundled = slot(manifest(4), 10), installed = slot(manifest(4), 4)))
    }

    @Test
    fun `a load of the current slot is not a fallback`() {
        assertFalse(BlocklistProvisioning.fellBackToPrevious(loadedVersion = 4, currentManifest = manifest(4)))
    }

    @Test
    fun `a load of another version than the current slot is a fallback`() {
        assertTrue(BlocklistProvisioning.fellBackToPrevious(loadedVersion = 3, currentManifest = manifest(4)))
    }

    @Test
    fun `a load with no current slot installed is a fallback`() {
        assertTrue(BlocklistProvisioning.fellBackToPrevious(loadedVersion = 3, currentManifest = null))
    }

    @Test
    fun `no loaded version is never a fallback`() {
        assertFalse(BlocklistProvisioning.fellBackToPrevious(loadedVersion = null, currentManifest = manifest(4)))
    }

    @Test
    fun `the fallback is recorded as its own event`() {
        assertEquals("list_fallback", TamperEvent.BLOCKLIST_FALLBACK.code)
    }
}
