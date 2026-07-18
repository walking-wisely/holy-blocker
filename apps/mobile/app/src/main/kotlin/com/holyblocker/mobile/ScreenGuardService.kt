package com.holyblocker.mobile

import android.accessibilityservice.AccessibilityService
import android.os.Build
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.holyblocker.mobile.policy.CoverState
import com.holyblocker.mobile.policy.GateOutcome
import com.holyblocker.mobile.policy.GuardDecision
import com.holyblocker.mobile.policy.NativeTextPolicy
import com.holyblocker.mobile.policy.ScanGate
import com.holyblocker.mobile.policy.ScreenIdentity
import com.holyblocker.mobile.policy.SettingsGuard
import com.holyblocker.mobile.policy.SettingsProfiles

/**
 * Layer 2's workhorse on Android: reads on-screen text from other apps, runs it
 * through `text-policy`, and covers the screen when the verdict says so.
 *
 * Deliberately thin. Everything decidable lives in [ScanGate] and is unit
 * tested on the JVM; this class only harvests text and applies the result, which
 * is the part that needs a real device to exercise.
 */
class ScreenGuardService : AccessibilityService() {

    private var policy: NativeTextPolicy? = null
    private var gate: ScanGate? = null
    private var overlay: OverlayController? = null
    private var settingsGuard: SettingsGuard? = null
    private var suspension: GuardSuspension? = null
    private var appliedSuspensionUntil = 0L

    override fun onServiceConnected() {
        super.onServiceConnected()
        val engine = NativeTextPolicy.withBuiltinDictionary()
        policy = engine
        gate = ScanGate(policy = engine, selfPackage = packageName)
        overlay = OverlayController(this)
        suspension = GuardSuspension(this)

        val profile = SettingsProfiles.forManufacturer(Build.MANUFACTURER)
        settingsGuard = SettingsGuard(
            profile = profile,
            selfPackage = packageName,
            selfLabel = getString(R.string.app_name),
        )
        if (profile == null) {
            // Not a failure to hide: on an unrecognised build the settings
            // screens cannot be identified, so the guard silently does nothing
            // and the onboarding screen says so.
            Log.w(TAG, "no settings profile for ${Build.MANUFACTURER}; screen guard inactive")
        }

        Log.i(TAG, "screen guard connected (settings profile=${profile?.name ?: "none"})")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        val gate = gate ?: return
        val overlay = overlay ?: return
        val packageName = event?.packageName?.toString() ?: return

        // The settings screens are checked first and are deliberately not behind
        // ScanGate's debounce: a 300 ms window on the one screen that removes the
        // guard is exactly the gap this is here to close.
        if (guardSettingsScreen(packageName, event, overlay)) return

        // Ask before harvesting: our own overlay fires events too, and the tree
        // walk below is the expensive part of this callback.
        if (!gate.shouldHarvest(packageName)) return

        val root = rootInActiveWindow ?: return
        val fragments = mutableListOf<String>()
        collectText(root, fragments, depth = 0)

        when (val outcome = gate.onScreenText(packageName, fragments, System.currentTimeMillis())) {
            is GateOutcome.Scanned -> {
                // Never log the scanned text itself — the action and score are
                // what a maintainer needs, and the content stays on the screen
                // it came from.
                Log.d(
                    TAG,
                    "scan pkg=$packageName action=${outcome.verdict.action} " +
                        "score=${outcome.verdict.score} cover=${outcome.cover}",
                )
                overlay.apply(outcome.cover)
            }

            is GateOutcome.Skipped -> Unit // leave the overlay as it is
        }
    }

    /**
     * Handles the screens that would remove the guard.
     *
     * Returns true when the event was consumed, so the caller skips the ordinary
     * content scan — the settings app is not content worth classifying, and
     * scanning it while backing out of it would be wasted work.
     */
    private fun guardSettingsScreen(
        packageName: String,
        event: AccessibilityEvent,
        overlay: OverlayController,
    ): Boolean {
        val guard = settingsGuard ?: return false
        if (!guard.watchesPackage(packageName)) {
            // Tell the guard we left before skipping the harvest: the re-fire
            // suppression is only safe if it knows a back action actually landed
            // somewhere else, otherwise tapping straight back into the settings
            // screen falls inside the window and is ignored.
            guard.onUnguardedScreen()
            return false
        }

        syncSuspension(guard)

        val root = rootInActiveWindow ?: return false
        val texts = mutableListOf<String>()
        val resourceIds = mutableSetOf<String>()
        collectIdentity(root, texts, resourceIds, depth = 0)

        val identity = ScreenIdentity(
            packageName = packageName,
            className = event.className?.toString(),
            resourceIds = resourceIds,
            texts = texts,
        )

        // Kept rather than removed after bring-up: this is how per-OEM
        // identifiers get collected, and the docs point contributors at it.
        Log.d(TAG, "settings screen class=${identity.className} ids=${resourceIds.take(12)}")

        return when (val decision = guard.evaluate(identity, System.currentTimeMillis())) {
            is GuardDecision.BackOut -> {
                Log.i(TAG, "backing out of ${decision.surface}")
                // Clear first: leaving is the action, and a cover left behind
                // would sit over whatever screen we land on.
                overlay.apply(CoverState.CLEAR)
                performGlobalAction(GLOBAL_ACTION_BACK)
                true
            }

            is GuardDecision.CoverOnly -> {
                // Backing out is not working — release navigation so a wrong
                // matcher cannot make the device unusable, and cover instead.
                Log.w(TAG, "back-out bound reached on ${decision.surface}; covering only")
                overlay.apply(CoverState.COVER)
                true
            }

            GuardDecision.Ignore -> false
        }
    }

    /** Picks up a release requested from the onboarding screen. */
    private fun syncSuspension(guard: SettingsGuard) {
        // Zero unless a request has cleared its cooldown, so a freshly tapped
        // request does not release anything.
        val until = suspension?.releasedUntilWallMillis() ?: return
        // Only on change: suspendUntil() also clears the back-out bound, so
        // calling it every event would reset the loop counter continuously.
        if (until != appliedSuspensionUntil) {
            appliedSuspensionUntil = until
            guard.suspendUntil(until)
            Log.i(TAG, "screen guard suspended until $until")
        }
    }

    /**
     * Depth-first walk collecting text and `getViewIdResourceName()`.
     *
     * Resource ids are what makes settings-screen matching survive a device in
     * another language, and they are only gathered here — for the settings app —
     * because the extra pass is not free.
     */
    private fun collectIdentity(
        node: AccessibilityNodeInfo?,
        texts: MutableList<String>,
        resourceIds: MutableSet<String>,
        depth: Int,
    ) {
        if (node == null || depth > MAX_DEPTH || texts.size >= MAX_FRAGMENTS) return

        node.text?.toString()?.let { if (it.isNotBlank()) texts += it }
        node.contentDescription?.toString()?.let { if (it.isNotBlank()) texts += it }
        node.viewIdResourceName?.let { resourceIds += it }

        for (i in 0 until node.childCount) {
            collectIdentity(node.getChild(i), texts, resourceIds, depth + 1)
            if (texts.size >= MAX_FRAGMENTS) return
        }
    }

    /**
     * Depth-first walk collecting `text` and `contentDescription`.
     *
     * Bounded by [MAX_DEPTH] and [MAX_FRAGMENTS] because this runs on the
     * UI-event path and some apps (web views especially) expose very deep trees;
     * an unbounded walk here would jank the foreground app.
     */
    private fun collectText(
        node: AccessibilityNodeInfo?,
        into: MutableList<String>,
        depth: Int,
    ) {
        if (node == null || depth > MAX_DEPTH || into.size >= MAX_FRAGMENTS) return

        node.text?.toString()?.let { if (it.isNotBlank()) into += it }
        node.contentDescription?.toString()?.let { if (it.isNotBlank()) into += it }

        for (i in 0 until node.childCount) {
            collectText(node.getChild(i), into, depth + 1)
            if (into.size >= MAX_FRAGMENTS) return
        }
    }

    override fun onInterrupt() {
        overlay?.apply(com.holyblocker.mobile.policy.CoverState.CLEAR)
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        teardown()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        teardown()
        super.onDestroy()
    }

    private fun teardown() {
        overlay?.destroy()
        overlay = null
        policy?.close()
        policy = null
        gate = null
        settingsGuard = null
        suspension = null
        appliedSuspensionUntil = 0
    }

    companion object {
        private const val TAG = "ScreenGuard"
        private const val MAX_DEPTH = 40
        private const val MAX_FRAGMENTS = 400
    }
}
