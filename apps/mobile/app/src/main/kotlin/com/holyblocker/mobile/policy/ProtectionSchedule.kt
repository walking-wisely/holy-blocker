package com.holyblocker.mobile.policy

/**
 * Where the user stands with respect to [SettingsGuard].
 *
 * Only [DISARMED] and [OFF] stop the guard. The two waiting phases exist so that
 * asking to be let out is visibly not the same thing as being let out.
 */
enum class ProtectionPhase {
    /** Never armed, or turned off. Nothing is blocked. */
    OFF,

    /** Protection is on. The removal screens are guarded. */
    ARMED,

    /** A disarm was requested and the cooldown is running. Still guarded. */
    DISARM_PENDING,

    /** The cooldown has passed; the user may now confirm. Still guarded. */
    DISARM_READY,

    /** Confirmed. The guard is off until the window runs out. */
    DISARMED,
}

data class ProtectionState(
    val phase: ProtectionPhase,
    /** Time left in the current phase; zero when nothing is running. */
    val remainingMillis: Long,
) {
    /** Whether [SettingsGuard] should be blocking the removal screens. */
    val guardActive: Boolean
        get() = phase == ProtectionPhase.ARMED ||
            phase == ProtectionPhase.DISARM_PENDING ||
            phase == ProtectionPhase.DISARM_READY
}

/**
 * Protection is a mode the user turns on, and turning it *off* is what costs.
 *
 * The guard does not block anything merely because the accessibility service is
 * running. It blocks because the user armed it, and it stops blocking when the
 * user has disarmed it — which is the only honest way to run a tool whose whole
 * value is that the user chose it. It also means the removal path is not
 * something to be defeated: the way to uninstall this app is to disarm it and
 * then uninstall it, through the front door.
 *
 * The shape, and why each step is there:
 *
 * - **Arming is one tap.** Protecting yourself should never be the slow
 *   direction.
 * - **Disarming is requested, not done.** The request starts a cooldown, and the
 *   guard keeps working throughout it. An urge does not survive fifteen minutes;
 *   a decision does. Releasing on the tap is what an earlier version of this did,
 *   and it made the exit path the fastest route through the guard rather than a
 *   considered way out.
 * - **The request must be confirmed.** Reaching the end of the cooldown does not
 *   disarm anything. Without the second tap a request made and forgotten would
 *   spend its window in the user's pocket, and the cooldown would have been paid
 *   for nothing.
 * - **An unconfirmed request expires.** Otherwise one cooldown is paid once and
 *   buys a disarm at any point in the future.
 * - **A disarm is a window, not a switch.** Long enough to deactivate device
 *   admin, open App info and uninstall without racing a clock; short enough that
 *   a user who gets distracted is protected again rather than permanently
 *   unprotected. The re-arm needs no action, which is the point.
 *
 * **Timing is monotonic and never touches the wall clock.**
 * `System.currentTimeMillis()` is user-settable and Settings' date screen is not
 * a guarded surface, so any wall-clock dependence here would reduce the cooldown
 * to "set the date forward an hour". `SystemClock.elapsedRealtime()` cannot be
 * set by the user and continues across deep sleep.
 *
 * Its one discontinuity is reboot, which resets it to zero. Both stored values
 * are therefore *start* timestamps rather than deadlines, and a stored value in
 * the future means the device restarted. Both such cases resolve to [ARMED] —
 * strict in both directions, and deliberately so for the disarm window: a stored
 * deadline read against a clock that just reset would look like a disarm with
 * hours left on it, making a reboot a way to stay unguarded indefinitely.
 *
 * Pure and unit tested; the storage edge is `ProtectionStore`.
 */
object ProtectionSchedule {

    /**
     * How long a disarm request waits before it can be confirmed.
     *
     * This value *is* the exit path's safety property. Shortening it below the
     * life of an urge turns the whole mechanism back into decoration.
     */
    const val COOLDOWN_MILLIS = 15 * 60_000L

    /** How long a request stays confirmable once the cooldown has passed. */
    const val READY_WINDOW_MILLIS = 5 * 60_000L

    /**
     * How long the guard stays off once a disarm is confirmed.
     *
     * Sized to the job the window exists for — deactivate device admin, App
     * info, uninstall — rather than to a round number. A window too short to
     * finish makes the user wait out the cooldown again having done nothing
     * wrong, which reads as the tool fighting them.
     */
    const val DISARM_WINDOW_MILLIS = 10 * 60_000L

    fun evaluate(
        armed: Boolean,
        disarmRequestedAtElapsed: Long?,
        disarmedAtElapsed: Long?,
        nowElapsed: Long,
    ): ProtectionState {
        if (!armed) return OFF

        // A *live* disarm outranks the request that produced it, which is
        // normally still stored. A spent one must not: the timestamp outlives
        // its window, and while it was consulted unconditionally it shadowed
        // every later request — one disarm per install, and the app could never
        // be removed a second time. Falling through is what makes the mode
        // repeatable.
        //
        // A timestamp in the future means the device rebooted; treat it as spent
        // rather than trusting a window measured against a clock that no longer
        // exists.
        disarmedAtElapsed?.let { disarmedAt ->
            val progress = nowElapsed - disarmedAt
            if (progress in 0 until DISARM_WINDOW_MILLIS) {
                return ProtectionState(ProtectionPhase.DISARMED, DISARM_WINDOW_MILLIS - progress)
            }
        }

        val requestedAt = disarmRequestedAtElapsed ?: return ARMED
        if (nowElapsed < requestedAt) return ARMED // reboot: void the request

        val progress = nowElapsed - requestedAt

        return when {
            progress < COOLDOWN_MILLIS ->
                ProtectionState(ProtectionPhase.DISARM_PENDING, COOLDOWN_MILLIS - progress)

            progress < COOLDOWN_MILLIS + READY_WINDOW_MILLIS ->
                ProtectionState(
                    ProtectionPhase.DISARM_READY,
                    COOLDOWN_MILLIS + READY_WINDOW_MILLIS - progress,
                )

            else -> ARMED
        }
    }

    private val OFF = ProtectionState(ProtectionPhase.OFF, 0)
    private val ARMED = ProtectionState(ProtectionPhase.ARMED, 0)
}
