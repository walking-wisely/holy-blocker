package com.holyblocker.mobile

import android.util.Log
import com.holyblocker.mobile.policy.ScreenCapture

/**
 * One captured frame, already downscaled and already judged worth looking at.
 *
 * The buffer is a private copy — the `Image` it came from is recycled the
 * instant the reader closes it — and it is expected to be consumed and dropped.
 * **Nothing in this app writes a frame anywhere**, and a sink that did would be
 * writing the user's whole screen to disk, which is the one thing this product
 * cannot do and remain what it claims to be.
 */
class CapturedFrame(
    val width: Int,
    val height: Int,
    /** Tightly packed RGBA_8888, [ScreenCapture.BYTES_PER_PIXEL] bytes per pixel. */
    val pixels: ByteArray,
    /** [ScreenCapture.dHash] of the frame, already computed by the gate. */
    val hash: Long,
    /** `SystemClock.elapsedRealtime()` when the frame was captured. */
    val capturedAtMillis: Long,
)

/**
 * Where an accepted frame goes.
 *
 * A seam, and a deliberately empty one for now: the image path this feeds —
 * perceptual hashing against a local set, OCR, and the ONNX classifier — is
 * `packages/image-sandbox`, which does not exist yet. Capturing frames is
 * useful before it lands (it is the half that needs a device to get right), and
 * an interface here is what keeps the platform work from having to be rewritten
 * when the analysis arrives.
 */
fun interface FrameSink {
    fun accept(frame: CapturedFrame)
}

/**
 * The stand-in sink: counts frames and says nothing about them.
 *
 * It logs a count and a size, never a pixel and never a hash — a hash is not
 * content, but it is a per-screen identifier, and putting a stream of them in
 * logcat builds exactly the browsing record this product refuses to keep.
 */
class CountingFrameSink : FrameSink {
    @Volatile
    var accepted: Long = 0L
        private set

    override fun accept(frame: CapturedFrame) {
        accepted++
        // The first one is logged on its own: it is the only evidence that the
        // projection, the reader and the gate are all connected, and waiting for
        // the sixtieth to find out is a minute of not knowing whether anything
        // works at all. scripts/smoke-test-capture.sh asserts on this line.
        if (accepted == 1L || accepted % LOG_EVERY == 0L) {
            Log.i(TAG, "analysed $accepted frames at ${frame.width}x${frame.height}")
        }
    }

    private companion object {
        const val TAG = "ScreenCapture"

        /** One line a minute or so at the gate's rate cap. */
        const val LOG_EVERY = 60L
    }
}
