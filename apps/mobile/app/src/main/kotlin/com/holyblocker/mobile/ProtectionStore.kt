package com.holyblocker.mobile

import android.content.Context
import android.os.SystemClock
import com.holyblocker.mobile.policy.ProtectionPhase
import com.holyblocker.mobile.policy.ProtectionSchedule
import com.holyblocker.mobile.policy.ProtectionState

/**
 * Storage for the protection mode: armed or not, and where a disarm has got to.
 *
 * `MainActivity` writes it and `ScreenGuardService` reads it. They are separate
 * objects in the same process, so this goes through `SharedPreferences` — which
 * also means the mode survives the service being restarted, and the guard comes
 * back armed after a reboot without the user doing anything.
 *
 * Only storage; every decision lives in [ProtectionSchedule] and is unit tested
 * without Android.
 *
 * **What is stored is a request, not a grant.** Clearing app data removes a
 * pending request and leaves the mode armed, so it makes the guard stricter,
 * never weaker — worth preserving, since clear-data is reachable from a screen
 * this app can only guard, not prevent.
 */
class ProtectionStore(context: Context) {

    private val prefs =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun state(nowElapsed: Long = SystemClock.elapsedRealtime()): ProtectionState =
        ProtectionSchedule.evaluate(
            armed = prefs.getBoolean(KEY_ARMED, false),
            disarmRequestedAtElapsed = prefs.getLong(KEY_REQUESTED_AT, NONE).takeIf { it != NONE },
            disarmedAtElapsed = prefs.getLong(KEY_DISARMED_AT, NONE).takeIf { it != NONE },
            nowElapsed = nowElapsed,
        )

    /**
     * Turns protection on. One tap, no wait — the slow direction is the other one.
     *
     * Clears any disarm in progress: arming while a window is open is the user
     * saying they are done, and leaving the timestamps in place would let the
     * remainder of that window be reclaimed later.
     */
    fun arm() {
        prefs.edit()
            .putBoolean(KEY_ARMED, true)
            .remove(KEY_REQUESTED_AT)
            .remove(KEY_DISARMED_AT)
            .apply()
    }

    /** Starts the cooldown. Re-requesting while one is live does not restart it. */
    fun requestDisarm(nowElapsed: Long = SystemClock.elapsedRealtime()) {
        if (state(nowElapsed).phase != ProtectionPhase.ARMED) return
        prefs.edit().putLong(KEY_REQUESTED_AT, nowElapsed).apply()
    }

    /**
     * Confirms a disarm that has served its cooldown, opening the window.
     *
     * Refuses in every other phase, which is what keeps the cooldown from being
     * skippable: a caller cannot disarm without a request that has matured.
     * Returns whether it took effect.
     */
    fun confirmDisarm(nowElapsed: Long = SystemClock.elapsedRealtime()): Boolean {
        if (state(nowElapsed).phase != ProtectionPhase.DISARM_READY) return false
        prefs.edit()
            .putLong(KEY_DISARMED_AT, nowElapsed)
            .remove(KEY_REQUESTED_AT)
            .apply()
        return true
    }

    /** Abandons a pending or ready request, leaving protection armed. */
    fun cancelDisarm() {
        prefs.edit().remove(KEY_REQUESTED_AT).apply()
    }

    companion object {
        private const val PREFS_NAME = "protection_mode"
        private const val KEY_ARMED = "armed"
        private const val KEY_REQUESTED_AT = "disarm_requested_at_elapsed"
        private const val KEY_DISARMED_AT = "disarmed_at_elapsed"
        private const val NONE = -1L
    }
}
