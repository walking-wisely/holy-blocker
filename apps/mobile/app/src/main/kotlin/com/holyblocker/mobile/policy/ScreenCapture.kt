package com.holyblocker.mobile.policy

/** Pixel dimensions of the surface frames are captured into. */
data class CaptureSize(val width: Int, val height: Int)

/**
 * The decision core of the screen-capture path: how large a frame to take, how
 * to reduce it to something comparable, and when the capture should be running
 * at all.
 *
 * **Everything here is arithmetic over a byte buffer, and that is deliberate.**
 * The capture path is the most invasive thing this app does — it is the whole
 * screen, of every app, including the ones it has no business reading — so the
 * parts that decide what happens to a frame are kept where they can be read and
 * tested without an emulator. `ScreenCaptureService` owns the projection, the
 * surface and the threads, and nothing else.
 *
 * Two properties follow from that and are load-bearing:
 *
 * - **A frame is never stored.** It is reduced to a 64-bit hash for comparison
 *   and handed to a sink that is expected to consume it immediately. Nothing
 *   here writes to disk, and nothing here logs a pixel.
 * - **The reduction is lossy on purpose.** [lumaGrid] throws away colour and all
 *   but 72 samples of the frame; what survives cannot reconstruct the screen,
 *   which is the right property for the one value that outlives the frame.
 *
 * ### Reference documents
 *
 * - [ITU-R BT.601-7](https://www.itu.int/rec/R-REC-BT.601) §2.5.1 — the luma
 *   coefficients used below.
 * - [`ImageReader`](https://developer.android.com/reference/android/media/ImageReader)
 *   and [`Image.Plane`](https://developer.android.com/reference/android/media/Image.Plane)
 *   — `rowStride` and `pixelStride`, which is why nothing here assumes a tightly
 *   packed buffer.
 * - [`PixelFormat#RGBA_8888`](https://developer.android.com/reference/android/graphics/PixelFormat#RGBA_8888)
 *   — the byte order the plane arrives in.
 */
object ScreenCapture {

    /**
     * Longest edge the capture surface is scaled to along the width.
     *
     * The frame exists to be hashed and, once `packages/image-sandbox` lands,
     * classified — neither wants a 1080p buffer. Capturing small is also the
     * cheapest privacy measure available: the pixels that never leave the GPU
     * cannot be mishandled later.
     */
    const val CAPTURE_MAX_WIDTH = 320

    /**
     * Width of the sample grid. One more than [HASH_GRID_HEIGHT] because
     * [dHash] compares each cell with its right neighbour, so a 9-wide row
     * yields 8 bits and eight rows fill a `Long` exactly.
     */
    const val HASH_GRID_WIDTH = 9

    /** Height of the sample grid. */
    const val HASH_GRID_HEIGHT = 8

    /** RGBA_8888 — four bytes per pixel, in R, G, B, A order. */
    const val BYTES_PER_PIXEL = 4

    // BT.601-7 §2.5.1 luma: Y = 0.299R + 0.587G + 0.114B, in 1/256ths so the
    // whole reduction stays in integer arithmetic on the frame path.
    private const val LUMA_RED = 77
    private const val LUMA_GREEN = 150
    private const val LUMA_BLUE = 29
    private const val LUMA_SHIFT = 8

    /**
     * Whether the capture should be running.
     *
     * Gated on [ProtectionState.guardActive], the same condition as the network
     * guard, so the whole product moves together — and, more importantly, so a
     * release window the user asked for really is a release. Capture that
     * carried on reading the screen through a disarm would make the one honest
     * exit this app offers a partial one.
     *
     * Consent is separate and cannot be inferred from anything stored: a
     * `MediaProjection` grant is per session, is not persistable, and is gone
     * the moment the user stops the projection from the system UI.
     */
    fun shouldRun(protection: ProtectionState, consentGranted: Boolean): Boolean =
        consentGranted && protection.guardActive

    /**
     * Scales a display down to the capture surface, preserving its shape.
     *
     * Never upscales — a small display is captured at its own size rather than
     * interpolated into a larger buffer that holds no more information.
     * Dimensions are forced even and non-zero: an odd or zero edge is rejected
     * by parts of the display pipeline, and an extreme aspect ratio rounds a
     * short edge to nothing without the floor.
     */
    fun captureSize(
        displayWidth: Int,
        displayHeight: Int,
        maxWidth: Int = CAPTURE_MAX_WIDTH,
    ): CaptureSize {
        require(displayWidth > 0 && displayHeight > 0) { "display has no area" }

        if (displayWidth <= maxWidth) {
            return CaptureSize(even(displayWidth), even(displayHeight))
        }
        val height = (displayHeight.toLong() * maxWidth / displayWidth).toInt()
        return CaptureSize(even(maxWidth), even(height))
    }

    private fun even(value: Int): Int = (value / 2 * 2).coerceAtLeast(2)

    /**
     * Reduces a captured plane to a small grid of average luma.
     *
     * Each cell is the mean brightness of its block of the frame, which is what
     * makes the hash below survive the noise a screen produces on its own —
     * antialiasing, a cursor, a progress bar — while still moving when the
     * content does.
     *
     * @param plane raw bytes of the [ImageReader][android.media.ImageReader]
     *   plane, which is **not** tightly packed: `rowStride` may exceed
     *   `width * pixelStride`, and reading past it shears the image.
     */
    fun lumaGrid(
        plane: ByteArray,
        width: Int,
        height: Int,
        rowStride: Int,
        pixelStride: Int = BYTES_PER_PIXEL,
        gridWidth: Int = HASH_GRID_WIDTH,
        gridHeight: Int = HASH_GRID_HEIGHT,
    ): IntArray {
        val grid = IntArray(gridWidth * gridHeight)

        for (cellY in 0 until gridHeight) {
            val top = cellY * height / gridHeight
            val bottom = (((cellY + 1) * height / gridHeight)).coerceAtLeast(top + 1).coerceAtMost(height)

            for (cellX in 0 until gridWidth) {
                val left = cellX * width / gridWidth
                val right = (((cellX + 1) * width / gridWidth)).coerceAtLeast(left + 1).coerceAtMost(width)

                var total = 0L
                var samples = 0
                for (y in top until bottom) {
                    var at = y * rowStride + left * pixelStride
                    for (x in left until right) {
                        // Alpha is skipped: a composited screenshot is opaque,
                        // and weighting by it would darken nothing but itself.
                        val r = plane[at].toInt() and 0xFF
                        val g = plane[at + 1].toInt() and 0xFF
                        val b = plane[at + 2].toInt() and 0xFF
                        total += (LUMA_RED * r + LUMA_GREEN * g + LUMA_BLUE * b) shr LUMA_SHIFT
                        samples++
                        at += pixelStride
                    }
                }
                grid[cellY * gridWidth + cellX] = if (samples == 0) 0 else (total / samples).toInt()
            }
        }
        return grid
    }

    /**
     * Difference hash of a luma grid: one bit per horizontal neighbour pair.
     *
     * A *gradient* comparison rather than an absolute one, which is the reason
     * to prefer it over an average hash here: the screen dims, auto-brightness
     * moves, a dark theme animates in, and none of those is a new screen. Only
     * the relationship between neighbouring cells is recorded, so a uniform
     * brightness change leaves the hash untouched.
     */
    fun dHash(
        grid: IntArray,
        gridWidth: Int = HASH_GRID_WIDTH,
        gridHeight: Int = HASH_GRID_HEIGHT,
    ): Long {
        require(grid.size >= gridWidth * gridHeight) { "grid is smaller than its stated size" }
        require((gridWidth - 1) * gridHeight <= Long.SIZE_BITS) { "grid yields more than 64 bits" }

        var hash = 0L
        var bit = 0
        for (y in 0 until gridHeight) {
            for (x in 0 until gridWidth - 1) {
                val here = grid[y * gridWidth + x]
                val next = grid[y * gridWidth + x + 1]
                if (here > next) hash = hash or (1L shl bit)
                bit++
            }
        }
        return hash
    }

    /** How many bits two frame hashes disagree on. */
    fun hammingDistance(a: Long, b: Long): Int = java.lang.Long.bitCount(a xor b)

    /**
     * Copies a plane into a tightly packed RGBA buffer.
     *
     * Only for frames that survived the gate: an `Image` is backed by a buffer
     * the reader reclaims on `close()`, so anything handed onward has to be a
     * copy, and doing it for every frame would copy a megabyte a second to throw
     * most of it away.
     */
    fun packRgba(
        plane: ByteArray,
        width: Int,
        height: Int,
        rowStride: Int,
        pixelStride: Int = BYTES_PER_PIXEL,
    ): ByteArray {
        val packed = ByteArray(width * height * BYTES_PER_PIXEL)
        var out = 0
        for (y in 0 until height) {
            var at = y * rowStride
            for (x in 0 until width) {
                packed[out] = plane[at]
                packed[out + 1] = plane[at + 1]
                packed[out + 2] = plane[at + 2]
                packed[out + 3] = plane[at + 3]
                out += BYTES_PER_PIXEL
                at += pixelStride
            }
        }
        return packed
    }
}
