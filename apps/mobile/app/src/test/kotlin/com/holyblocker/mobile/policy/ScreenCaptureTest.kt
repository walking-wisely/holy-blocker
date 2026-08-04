package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ScreenCaptureTest {

    // -- shouldRun -----------------------------------------------------------

    @Test
    fun `capture runs only with consent and an armed guard`() {
        val armed = ProtectionState(ProtectionPhase.ARMED, remainingMillis = 0)
        assertTrue(ScreenCapture.shouldRun(armed, consentGranted = true))
        assertFalse(ScreenCapture.shouldRun(armed, consentGranted = false))
    }

    @Test
    fun `capture stops when protection is off, consent or not`() {
        val off = ProtectionState(ProtectionPhase.OFF, remainingMillis = 0)
        assertFalse(ScreenCapture.shouldRun(off, consentGranted = true))
    }

    @Test
    fun `capture stops during an open release window`() {
        // DISARMED is the window the user asked for. Reading the screen through
        // it would make the release the one thing it must not be: partial.
        val released = ProtectionState(ProtectionPhase.DISARMED, remainingMillis = 60_000)
        assertFalse(ScreenCapture.shouldRun(released, consentGranted = true))
    }

    // -- captureSize ---------------------------------------------------------

    @Test
    fun `capture size scales the display down and keeps its shape`() {
        val size = ScreenCapture.captureSize(1080, 2400, maxWidth = 320)

        assertEquals(320, size.width)
        // 2400 / 1080 * 320 = 711.1, rounded down to an even 710.
        assertEquals(710, size.height)
        assertTrue("aspect drift", kotlin.math.abs(size.height / size.width.toDouble() - 2400 / 1080.0) < 0.01)
    }

    @Test
    fun `capture size never upscales a display smaller than the cap`() {
        val size = ScreenCapture.captureSize(240, 320, maxWidth = 320)

        assertEquals(240, size.width)
        assertEquals(320, size.height)
    }

    @Test
    fun `capture size is always even and never zero`() {
        // ImageReader and the display pipeline both dislike odd dimensions, and
        // a zero from an extreme aspect ratio would fail the surface outright.
        val wide = ScreenCapture.captureSize(4000, 3, maxWidth = 320)

        assertEquals(0, wide.width % 2)
        assertEquals(0, wide.height % 2)
        assertTrue(wide.height >= 2)
    }

    // -- lumaGrid ------------------------------------------------------------

    @Test
    fun `luma weights the channels the way BT 601 does`() {
        // One pure-red pixel: 0.299 * 255 = 76.2.
        val grid = ScreenCapture.lumaGrid(
            plane = byteArrayOf(0xFF.toByte(), 0, 0, 0xFF.toByte()),
            width = 1,
            height = 1,
            rowStride = 4,
            pixelStride = 4,
            gridWidth = 1,
            gridHeight = 1,
        )

        assertTrue("expected ~76, got ${grid[0]}", grid[0] in 74..78)
    }

    @Test
    fun `luma ignores the alpha channel`() {
        val opaque = ScreenCapture.lumaGrid(
            plane = byteArrayOf(0x10, 0x20, 0x30, 0xFF.toByte()),
            width = 1, height = 1, rowStride = 4, pixelStride = 4,
            gridWidth = 1, gridHeight = 1,
        )
        val transparent = ScreenCapture.lumaGrid(
            plane = byteArrayOf(0x10, 0x20, 0x30, 0x00),
            width = 1, height = 1, rowStride = 4, pixelStride = 4,
            gridWidth = 1, gridHeight = 1,
        )

        assertEquals(opaque[0], transparent[0])
    }

    @Test
    fun `row padding is skipped rather than sampled`() {
        // The hazard this test exists for: ImageReader hands back a plane whose
        // rowStride exceeds width * pixelStride, and reading it as if it were
        // tightly packed shears the image diagonally.
        val width = 2
        val height = 2
        val padding = 8
        val rowStride = width * 4 + padding

        val plane = ByteArray(rowStride * height) { 0x00 }
        // Row 0 is black and row 1 is white. The padding after the black row is
        // filled white, so a reader that walks the plane as if it were tightly
        // packed samples it and gets a different answer.
        for (x in 0 until width) {
            val at = rowStride + x * 4
            plane[at] = 0xFF.toByte()
            plane[at + 1] = 0xFF.toByte()
            plane[at + 2] = 0xFF.toByte()
        }
        for (i in 0 until padding) {
            plane[width * 4 + i] = 0xFF.toByte()
        }

        val grid = ScreenCapture.lumaGrid(
            plane, width, height, rowStride, pixelStride = 4,
            gridWidth = 1, gridHeight = 2,
        )

        assertEquals(0, grid[0])
        assertTrue("bottom row should be white, got ${grid[1]}", grid[1] > 250)
    }

    @Test
    fun `each grid cell averages its block`() {
        // 2x1 image, half black and half white, sampled into one cell.
        val plane = ByteArray(8)
        plane[4] = 0xFF.toByte()
        plane[5] = 0xFF.toByte()
        plane[6] = 0xFF.toByte()

        val grid = ScreenCapture.lumaGrid(
            plane, width = 2, height = 1, rowStride = 8, pixelStride = 4,
            gridWidth = 1, gridHeight = 1,
        )

        assertTrue("expected mid grey, got ${grid[0]}", grid[0] in 120..135)
    }

    // -- dHash ---------------------------------------------------------------

    @Test
    fun `a flat image hashes to zero`() {
        val grid = IntArray(ScreenCapture.HASH_GRID_WIDTH * ScreenCapture.HASH_GRID_HEIGHT) { 42 }

        assertEquals(0L, ScreenCapture.dHash(grid))
    }

    @Test
    fun `a left-to-right fall sets every bit`() {
        val w = ScreenCapture.HASH_GRID_WIDTH
        val h = ScreenCapture.HASH_GRID_HEIGHT
        val grid = IntArray(w * h) { i -> 255 - (i % w) * 20 }

        // 8 comparisons per row over 8 rows fills the long.
        assertEquals(-1L, ScreenCapture.dHash(grid))
    }

    @Test
    fun `two different screens hash differently`() {
        val w = ScreenCapture.HASH_GRID_WIDTH
        val h = ScreenCapture.HASH_GRID_HEIGHT
        val rising = IntArray(w * h) { i -> (i % w) * 20 }
        val falling = IntArray(w * h) { i -> 255 - (i % w) * 20 }

        assertNotEquals(ScreenCapture.dHash(rising), ScreenCapture.dHash(falling))
    }

    @Test
    fun `the hash is a gradient comparison, not an absolute one`() {
        // A uniform brightness change — the screen dimming — must not read as a
        // different screen.
        val w = ScreenCapture.HASH_GRID_WIDTH
        val h = ScreenCapture.HASH_GRID_HEIGHT
        val bright = IntArray(w * h) { i -> 200 - (i % w) * 10 }
        val dim = IntArray(w * h) { i -> 100 - (i % w) * 10 }

        assertEquals(ScreenCapture.dHash(bright), ScreenCapture.dHash(dim))
    }

    // -- hammingDistance -----------------------------------------------------

    @Test
    fun `hamming distance counts differing bits`() {
        assertEquals(0, ScreenCapture.hammingDistance(0x0FL, 0x0FL))
        assertEquals(4, ScreenCapture.hammingDistance(0x0FL, 0x00L))
        assertEquals(64, ScreenCapture.hammingDistance(0L, -1L))
    }

    // -- packRgba ------------------------------------------------------------

    @Test
    fun `packing strips row padding and keeps channel order`() {
        val width = 2
        val height = 2
        val rowStride = width * 4 + 4
        val plane = ByteArray(rowStride * height) { 0x7F }
        for (y in 0 until height) {
            for (x in 0 until width) {
                val at = y * rowStride + x * 4
                plane[at] = (y * 2 + x).toByte() // R carries the pixel index
                plane[at + 1] = 0x11
                plane[at + 2] = 0x22
                plane[at + 3] = 0x33
            }
        }

        val packed = ScreenCapture.packRgba(plane, width, height, rowStride, pixelStride = 4)

        assertEquals(width * height * 4, packed.size)
        for (i in 0 until width * height) {
            assertEquals(i.toByte(), packed[i * 4])
            assertEquals(0x11.toByte(), packed[i * 4 + 1])
            assertEquals(0x22.toByte(), packed[i * 4 + 2])
            assertEquals(0x33.toByte(), packed[i * 4 + 3])
        }
    }
}
