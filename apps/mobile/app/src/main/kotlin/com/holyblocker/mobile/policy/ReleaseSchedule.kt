package com.holyblocker.mobile.policy

enum class ReleasePhase {
    /** No request, or one that has expired. */
    IDLE,

    /** Requested; the cooldown is still running. */
    PENDING,

    /** The cooldown has passed and the guard is released. */
    OPEN,
}

data class ReleaseState(
    val phase: ReleasePhase,
    /** Time left in the current phase; zero when [ReleasePhase.IDLE]. */
    val remainingMillis: Long,
)

/**
 * When a requested release of the guard actually takes effect.
 *
 * The first version of the exit path released the guard the instant the button
 * was tapped, which made it the fastest way *through* the guard rather than a
 * considered exit: open the app, tap release, tap the accessibility shortcut
 * sitting directly beneath it, toggle off. Four taps and no research — faster
 * than any of the bypasses the guard was built to close. Its own documentation
 * claimed "an impulse does not survive a wait" while implementing no wait at
 * all.
 *
 * So a request and its release are separated. The cooldown is the mechanism: an
 * urge does not survive fifteen minutes, and a considered decision does.
 *
 * **Timing is monotonic and never touches the wall clock.**
 * `System.currentTimeMillis()` is user-settable and Settings' date screen is not
 * a guarded surface, so any wall-clock dependence here would reduce the cooldown
 * to "set the date forward an hour". `SystemClock.elapsedRealtime()` cannot be
 * set by the user and continues across deep sleep.
 *
 * Its one discontinuity is reboot, which resets it to zero. A stored value
 * greater than now therefore means the device restarted, and the request is
 * voided rather than guessed at — strict, and it keeps the wall clock out of the
 * decision entirely. The cost is that a reboot mid-cooldown means requesting
 * again, which errs the safe way.
 */
object ReleaseSchedule {

    /**
     * How long a request waits before it opens.
     *
     * This value *is* the exit path's safety property. Shortening it below the
     * life of an urge turns the whole mechanism back into decoration.
     */
    const val COOLDOWN_MILLIS = 15 * 60_000L

    /** How long the guard stays released once open. */
    const val WINDOW_MILLIS = 60_000L

    fun evaluate(requestedAtElapsed: Long?, nowElapsed: Long): ReleaseState {
        if (requestedAtElapsed == null) return IDLE

        // Monotonic clock ran backwards: the device rebooted. Void the request.
        if (nowElapsed < requestedAtElapsed) return IDLE

        val progress = nowElapsed - requestedAtElapsed

        return when {
            progress < COOLDOWN_MILLIS ->
                ReleaseState(ReleasePhase.PENDING, COOLDOWN_MILLIS - progress)

            progress < COOLDOWN_MILLIS + WINDOW_MILLIS ->
                ReleaseState(ReleasePhase.OPEN, COOLDOWN_MILLIS + WINDOW_MILLIS - progress)

            // Expired rather than still open: otherwise one cooldown buys
            // unlimited later access.
            else -> IDLE
        }
    }

    private val IDLE = ReleaseState(ReleasePhase.IDLE, 0)
}
