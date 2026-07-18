package com.holyblocker.mobile

import android.accessibilityservice.AccessibilityService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.holyblocker.mobile.admin.HolyBlockerAdminReceiver
import com.holyblocker.mobile.policy.CoverState
import com.holyblocker.mobile.policy.GateOutcome
import com.holyblocker.mobile.policy.GuardDecision
import com.holyblocker.mobile.policy.NativeTextPolicy
import com.holyblocker.mobile.policy.RescanSchedule
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

    private val rescan = RescanSchedule()
    private val handler = Handler(Looper.getMainLooper())

    /**
     * The deferred second look at a screen whose events fired before it had
     * finished populating. See [RescanSchedule] for why this is necessary.
     */
    // Type is explicit because the body reposts itself, which Kotlin cannot
    // infer through.
    private val rescanTask: Runnable = Runnable {
        val matched = evaluateCurrentScreen()
        if (!matched) {
            rescan.onRescanMissed()?.let { handler.postDelayed(rescanTask, it) }
        }
    }

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
            // Queried per event, not captured: the admin is normally activated
            // from onboarding well after this service connects, and until it is
            // the activation screen must stay reachable.
            isDeviceAdminActive = { HolyBlockerAdminReceiver.isActive(this) },
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
            cancelRescan()
            return false
        }

        syncSuspension(guard)

        val identity = currentScreenIdentity(packageName, event.className?.toString())
        val acted = identity != null && applyDecision(guard, identity, overlay)

        if (acted) {
            cancelRescan()
            return true
        }

        // Two different misses, both needing the same answer: look again later.
        //
        // Either nothing matched — the tree may still be filling in — or there
        // was no tree to walk at all. The second is not an edge case:
        // rootInActiveWindow is routinely null on the window-state event for a
        // screen that is still being brought forward, and on an android-36
        // emulator re-entering the settings task delivers exactly one event, in
        // exactly that state. Returning here without arming the re-look is what
        // left the device-admin list unguarded indefinitely.
        //
        // Each event restarts the wait, so a render burst costs one deferred
        // look rather than one per event.
        rescan.onWatchedEvent()?.let { scheduleRescan(it) }
        return acted
    }

    /**
     * Re-evaluates whatever is on screen now, with no event to describe it.
     *
     * Driven by [rescanTask] to catch screens that finished populating after
     * their events had already fired. Returns whether the guard acted.
     */
    private fun evaluateCurrentScreen(): Boolean {
        val guard = settingsGuard ?: return false
        val overlay = overlay ?: return false
        val packageName = foregroundPackage() ?: return false

        if (!guard.watchesPackage(packageName)) {
            guard.onUnguardedScreen()
            return false
        }

        syncSuspension(guard)

        // className is null on purpose: there is no event to carry it, and the
        // root node reports a view class rather than the activity. That costs
        // nothing worth having — the class is the unreliable signal here, while
        // resource ids and the self-mention catch-all both still apply.
        val identity = currentScreenIdentity(packageName, className = null) ?: return false
        return applyDecision(guard, identity, overlay)
    }

    /**
     * The node tree for [packageName], preferring the active window.
     *
     * `rootInActiveWindow` alone is not enough. Returning to a backgrounded
     * settings task leaves it null indefinitely — measured on an android-36
     * emulator, still null two seconds after the screen was fully drawn and
     * interactive — so a guard that only reads it never sees that screen at all.
     * Enumerating windows finds the tree that is genuinely there.
     *
     * `flagRetrieveInteractiveWindows` is already set in
     * `accessibility_service_config.xml`, so this costs no new capability.
     */
    /**
     * Which app the user is looking at, for a re-look that has no event to ask.
     *
     * Falls back to the window list for the same reason as [rootFor]: on a
     * restored task `rootInActiveWindow` is null, and that is exactly when the
     * deferred look needs to work.
     */
    private fun foregroundPackage(): String? =
        rootInActiveWindow?.packageName?.toString()
            ?: windows.firstOrNull { it.isActive }?.root?.packageName?.toString()

    private fun rootFor(packageName: String): AccessibilityNodeInfo? {
        rootInActiveWindow
            ?.takeIf { it.packageName?.toString() == packageName }
            ?.let { return it }

        return windows.asSequence()
            .mapNotNull { it.root }
            .firstOrNull { it.packageName?.toString() == packageName }
    }

    private fun currentScreenIdentity(packageName: String, className: String?): ScreenIdentity? {
        val root = rootFor(packageName) ?: return null
        val texts = mutableListOf<String>()
        val resourceIds = mutableSetOf<String>()
        collectIdentity(root, texts, resourceIds, depth = 0)

        // Kept rather than removed after bring-up: this is how per-OEM
        // identifiers get collected, and the docs point contributors at it.
        // texts is a count, never the strings: the screen's contents stay on the
        // screen. It is here because an empty harvest on a visibly populated
        // screen is the signature of the node tree lagging the display, and
        // without it that looks identical to a screen that simply did not match.
        Log.d(
            TAG,
            "settings screen class=$className texts=${texts.size} ids=${resourceIds.take(12)}",
        )

        // An empty harvest on a screen the user can plainly read is the open bug
        // (backlog.md item 1b). Three hypotheses produce it and they need
        // different fixes, so dump what tells them apart — but only on the
        // failing case, since this walks the tree a second time.
        if (texts.isEmpty()) logEmptyHarvest(packageName, root)

        return ScreenIdentity(
            packageName = packageName,
            className = className,
            resourceIds = resourceIds,
            texts = texts,
        )
    }

    /**
     * Diagnostic for an empty harvest on a populated screen — backlog item 1(b).
     *
     * Discriminates the three candidate causes in one dump:
     *
     *  - **Wrong window.** More than one window for [packageName] means [rootFor]
     *    picking the first match is a real suspect. Exactly one means it is not,
     *    and window-resolution work would be wasted.
     *  - **Filtered subtree.** `declared` counts children the tree says exist;
     *    `fetched` counts the ones `getChild` actually returned. A gap is the
     *    signature of nodes filtered out of this service's view — the case
     *    `flagIncludeNotImportantViews` would address, and the reason a
     *    `uiautomator` dump seeing the row proves nothing about what we can see.
     *  - **Genuinely absent.** `declared == fetched` with no text means the rows
     *    are not in the tree we were handed at all, and neither of the above
     *    helps.
     *
     * No text and no window titles are logged, only shape — the screen's contents
     * stay on the screen, same rule as the harvest log above.
     */
    private fun logEmptyHarvest(packageName: String, root: AccessibilityNodeInfo) {
        val matching = windows.filter { it.root?.packageName?.toString() == packageName }
        val shape = matching.joinToString { "id=${it.id} active=${it.isActive} focused=${it.isFocused} type=${it.type}" }

        var declared = 0
        var fetched = 0
        fun walk(node: AccessibilityNodeInfo?, depth: Int) {
            if (node == null || depth > MAX_DEPTH) return
            declared += node.childCount
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                fetched++
                walk(child, depth + 1)
            }
        }
        walk(root, depth = 0)

        Log.w(
            TAG,
            "empty harvest pkg=$packageName windows=${matching.size} [$shape] " +
                "rootChildren=${root.childCount} declared=$declared fetched=$fetched",
        )
    }

    /** Applies a guard decision. Returns whether it acted. */
    private fun applyDecision(
        guard: SettingsGuard,
        identity: ScreenIdentity,
        overlay: OverlayController,
    ): Boolean = when (val decision = guard.evaluate(identity, System.currentTimeMillis())) {
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

    private fun scheduleRescan(delayMillis: Long) {
        handler.removeCallbacks(rescanTask)
        handler.postDelayed(rescanTask, delayMillis)
    }

    private fun cancelRescan() {
        handler.removeCallbacks(rescanTask)
        rescan.reset()
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
        // Before the overlay and policy go: a queued re-look would otherwise run
        // against a torn-down service and a closed policy engine.
        cancelRescan()
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
