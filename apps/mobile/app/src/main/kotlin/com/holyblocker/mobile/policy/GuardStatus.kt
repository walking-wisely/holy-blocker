package com.holyblocker.mobile.policy

/**
 * Everything the status service can observe about the guard at one moment.
 *
 * Three independent facts, and the point of gathering them in one place is that
 * they disagree: the mode can read `ARMED` while the accessibility service that
 * enforces it has been switched off underneath, and a recognised mode on an
 * unrecognised device guards nothing. The notification is only worth having if
 * it says which of those is true.
 */
data class GuardConditions(
    /** Whether `ScreenGuardService` is enabled in `Settings.Secure`. */
    val serviceEnabled: Boolean,
    val protection: ProtectionState,
    /** Whether [SettingsProfiles] has identifiers for this build. */
    val deviceRecognised: Boolean,
)

/** Whether the guard is doing what the user asked of it. */
enum class GuardHealth {
    /** Armed, and the service that enforces it is running. */
    PROTECTING,

    /**
     * Armed, and the service is not running.
     *
     * The state the status service exists to catch. Nothing inside
     * `ScreenGuardService` can report this — by the time it is true, that service
     * is gone. See `backlog.md`, "Cannot be closed at Device Admin level": an
     * `adb` disable, a guest session, or a settings screen an OEM build hides
     * from the guard all end here, and a still-alive process noticing is the only
     * response available.
     */
    UNPROTECTED,

    /**
     * Nothing is armed, or a release the user asked for is running.
     *
     * Not a fault, and deliberately not reported as one: turning the service off
     * during a release window is the supported way out of this app.
     */
    IDLE,
}

/**
 * What the ongoing notification should say, in priority order.
 *
 * An enum rather than a string so the choice is testable without Android. The
 * service turns each one into copy.
 */
enum class StatusMessage {
    /** Armed, but nothing is enforcing it. */
    GUARD_NOT_RUNNING,

    DISARM_PENDING,
    DISARM_READY,
    DISARMED,

    /** Armed and running, but the settings screens are unguarded here. */
    DEVICE_UNVERIFIED,

    PROTECTING,

    /** Protection is off. The content scan may still be running. */
    IDLE,
}

/**
 * The decision core of the foreground status service.
 *
 * **The foreground service is not what keeps the guard alive.** An
 * `AccessibilityService` is system-bound and is rebound on boot for as long as it
 * stays enabled, so this neither makes the guard harder to kill nor is needed for
 * it to survive a restart. What it provides is the half of tamper detection the
 * guard cannot do for itself: a process that is still running *after* the guard
 * stops, and can therefore notice that it did.
 *
 * Pure and unit tested; the platform edge is `GuardStatusService`.
 */
object GuardStatus {

    /**
     * How often the conditions are re-read.
     *
     * A ceiling and a floor, both real. Too slow and "the guard was switched off"
     * is again something only discovered at the next connect, which is what the
     * accessibility service already manages on its own. Too fast and this wakes
     * to read `SharedPreferences` and a secure setting for no gain — the change
     * it is watching for is a human action, not a fast-moving signal, and the
     * setting change also arrives through a `ContentObserver`, for which this
     * poll is the backstop rather than the mechanism.
     */
    const val CHECK_INTERVAL_MILLIS = 60_000L

    fun health(conditions: GuardConditions): GuardHealth = when {
        !conditions.protection.guardActive -> GuardHealth.IDLE
        conditions.serviceEnabled -> GuardHealth.PROTECTING
        else -> GuardHealth.UNPROTECTED
    }

    /**
     * What to write to the tamper log for a change in health, if anything.
     *
     * Only the fall into [GuardHealth.UNPROTECTED], and only on the edge. Every
     * other transition here already has a writer — `ScreenGuardService` records
     * its own connect and unbind, `ProtectionStore` records each mode transition
     * — and a second entry for the same fact makes the record harder to read
     * rather than fuller. Staying unprotected is not re-recorded: this is polled,
     * so an evening with the service off would otherwise write an entry a minute
     * and push the history that matters past the cap.
     *
     * @param previous the last observed health, or null on the first check of
     *   this process. Null is treated as a fall rather than as healthy: a device
     *   that comes up with protection armed and the service disabled has a gap,
     *   and that is exactly the shape an `adb` disable leaves after a reboot.
     */
    fun noticeOn(previous: GuardHealth?, current: GuardHealth): TamperEvent? =
        if (current == GuardHealth.UNPROTECTED && previous != current) {
            TamperEvent.GUARD_UNPROTECTED
        } else {
            null
        }

    /**
     * Priority is the whole content of this function.
     *
     * [StatusMessage.GUARD_NOT_RUNNING] wins outright — it is the one state the
     * user has to be told about, and a countdown shown over it would describe a
     * release from a guard that has already stopped. The release phases come
     * next, being the thing the user is actively waiting on. The unverified-device
     * notice only outranks the plain armed message: it is static and is on the
     * onboarding screen too, but claiming protection on a build where the settings
     * screens are unguarded is the same failure the onboarding copy avoids.
     */
    fun message(conditions: GuardConditions): StatusMessage = when {
        health(conditions) == GuardHealth.UNPROTECTED -> StatusMessage.GUARD_NOT_RUNNING

        conditions.protection.phase == ProtectionPhase.DISARM_PENDING -> StatusMessage.DISARM_PENDING
        conditions.protection.phase == ProtectionPhase.DISARM_READY -> StatusMessage.DISARM_READY
        conditions.protection.phase == ProtectionPhase.DISARMED -> StatusMessage.DISARMED
        conditions.protection.phase == ProtectionPhase.OFF -> StatusMessage.IDLE

        !conditions.deviceRecognised -> StatusMessage.DEVICE_UNVERIFIED
        else -> StatusMessage.PROTECTING
    }

    /**
     * Whether the status service still has anything to report.
     *
     * It stops once protection is off *and* the accessibility service is
     * disabled: there is then no guard to watch and nothing armed to fall out of,
     * so the ongoing notification would be a permanent claim on the shade with
     * nothing behind it. Note it keeps running while the service scans with
     * protection off — an accessibility service reading the screen is exactly
     * what an ongoing notification should make visible.
     */
    fun shouldKeepRunning(conditions: GuardConditions): Boolean =
        conditions.serviceEnabled || conditions.protection.phase != ProtectionPhase.OFF
}
