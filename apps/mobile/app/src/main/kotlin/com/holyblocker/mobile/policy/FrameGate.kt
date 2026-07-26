package com.holyblocker.mobile.policy

/** Why a captured frame did not reach the analyser. */
enum class FrameSkipReason {
    /** Too soon after the last analysed frame. */
    TOO_SOON,

    /** Close enough to the last analysed frame that the verdict cannot differ. */
    UNCHANGED,
}

sealed interface FrameOutcome {
    /** Worth a pass over the pixels. */
    data object Analyse : FrameOutcome

    data class Skipped(val reason: FrameSkipReason) : FrameOutcome
}

/**
 * Decides which captured frames are worth analysing.
 *
 * `MediaProjection` delivers a frame per composition — sixty or a hundred and
 * twenty a second while anything on screen moves — and the image path behind
 * this is the most expensive thing on the device: a downscale, an OCR pass, and
 * a model. Running it per frame would drain the battery of a phone whose owner
 * is trying to keep this app installed, which makes the throttle a durability
 * feature rather than an optimisation.
 *
 * Two independent brakes, in the order they are cheapest to apply:
 *
 * - **A hard rate cap.** No more than one analysis per
 *   [minIntervalMillis], whatever is on screen. This is the one that bounds the
 *   cost; the dedupe below is best-effort by comparison.
 * - **A change threshold.** A frame within [changeThresholdBits] of the last
 *   analysed one is the same screen with a blinking cursor, and cannot produce a
 *   different verdict.
 *
 * **A dropped frame never becomes the baseline**, and that is the subtle part.
 * Comparing each frame against its immediate predecessor would let a slow
 * dissolve — one that moves a few bits per frame and the whole screen over two
 * seconds — through as an unbroken run of "unchanged". The baseline is the last
 * frame actually *analysed*, so drift accumulates against a fixed point.
 *
 * Not thread-safe: driven from the single [android.media.ImageReader] callback
 * thread, and built to be called from it.
 */
class FrameGate(
    private val minIntervalMillis: Long = DEFAULT_MIN_INTERVAL_MILLIS,
    private val changeThresholdBits: Int = DEFAULT_CHANGE_THRESHOLD_BITS,
) {
    private var lastAnalysedHash: Long? = null
    private var lastAnalysedAtMillis = 0L

    /**
     * @param hash [ScreenCapture.dHash] of the frame.
     * @param nowMillis a monotonic clock — `SystemClock.elapsedRealtime()` at the
     *   edge. Wall time is user-settable, and a clock moved backwards mid-session
     *   would park the gate for the length of the jump.
     */
    fun onFrame(hash: Long, nowMillis: Long): FrameOutcome {
        val baseline = lastAnalysedHash

        // The first frame of a session is always analysed: there is nothing to
        // compare it against, and it is the screen the user consented over.
        if (baseline != null) {
            if (nowMillis - lastAnalysedAtMillis < minIntervalMillis) {
                return FrameOutcome.Skipped(FrameSkipReason.TOO_SOON)
            }
            if (ScreenCapture.hammingDistance(hash, baseline) < changeThresholdBits) {
                return FrameOutcome.Skipped(FrameSkipReason.UNCHANGED)
            }
        }

        lastAnalysedHash = hash
        lastAnalysedAtMillis = nowMillis
        return FrameOutcome.Analyse
    }

    /**
     * Forgets the session.
     *
     * Called when the projection stops. Consent is per session, so the next one
     * is a fresh grant over a screen this gate has no reason to assume anything
     * about — and carrying a baseline across the gap would silently skip the
     * first frame of the new session if it happened to match the last of the old.
     */
    fun reset() {
        lastAnalysedHash = null
        lastAnalysedAtMillis = 0
    }

    companion object {
        /**
         * The rate cap.
         *
         * One second is slower than a scroll and faster than a decision: the
         * thing being caught is content that stays on screen, and content that
         * does not stay is content the user has already scrolled past.
         */
        const val DEFAULT_MIN_INTERVAL_MILLIS = 1_000L

        /**
         * How many of the 64 hash bits must move before a frame counts as a new
         * screen.
         *
         * Low, deliberately. The cost of an unnecessary analysis is a model pass;
         * the cost of a missed one is the thing this product exists to prevent,
         * so the threshold sits where it rejects cursor blink and clock ticks and
         * little else.
         */
        const val DEFAULT_CHANGE_THRESHOLD_BITS = 6
    }
}
