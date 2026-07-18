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
import com.holyblocker.mobile.policy.WindowCandidate
import com.holyblocker.mobile.policy.WindowResolver

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
        collectText(root, fragments, depth = 0, budget = NodeBudget())

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

        // windowId is the point of backlog item 2: the event names the window
        // that actually changed, which in split screen is the only way to tell
        // the two settings-adjacent panes apart.
        val identity = currentScreenIdentity(
            packageName,
            event.className?.toString(),
            eventWindowId = event.windowId,
        )
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
        //
        // eventWindowId is null for the same reason, which is why WindowResolver
        // needs the focus/active fallback: this path has no event to name a
        // window and must choose one on the display's own evidence.
        val identity = currentScreenIdentity(
            packageName,
            className = null,
            eventWindowId = null,
        ) ?: return false
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

    /**
     * The window to evaluate, paired with its root — see [WindowResolver].
     *
     * `rootInActiveWindow` is no longer a shortcut here even when it matches the
     * package: it says nothing about *which* window it came from, and picking a
     * window is the whole point. Enumerating gives ids and focus for every
     * candidate, which is what [WindowResolver] needs and what the decision
     * downstream needs.
     */
    private fun resolveWindow(packageName: String, eventWindowId: Int?): ResolvedWindow? {
        val roots = windows.mapNotNull { window ->
            window.root?.let { root -> window to root }
        }
        val candidates = roots.map { (window, root) ->
            WindowCandidate(
                id = window.id,
                packageName = root.packageName?.toString(),
                isActive = window.isActive,
                isFocused = window.isFocused,
            )
        }

        val chosen = WindowResolver.choose(candidates, packageName, eventWindowId)
            // The window list can come back empty on a task being restored, which
            // is case 1(a) all over again. rootInActiveWindow is the last resort
            // rather than the first choice, and it carries no focus information —
            // treat it as focused, since a single active window is.
            ?: return rootInActiveWindow
                ?.takeIf { it.packageName?.toString() == packageName }
                ?.let { ResolvedWindow(root = it, isFocused = true) }

        val root = roots.first { (window, _) -> window.id == chosen.id }.second
        return ResolvedWindow(root = root, isFocused = chosen.isFocused)
    }

    /** A window the guard has decided to evaluate. */
    private data class ResolvedWindow(val root: AccessibilityNodeInfo, val isFocused: Boolean)

    private fun currentScreenIdentity(
        packageName: String,
        className: String?,
        eventWindowId: Int?,
    ): ScreenIdentity? {
        val resolved = resolveWindow(packageName, eventWindowId) ?: run {
            // Not silent, deliberately. Case 1(a) was invisible for as long as it
            // was because every log sat behind `root != null`, so a guard that
            // never saw the screen looked identical to one that saw it and found
            // nothing to match. This is the same failure one layer up from
            // logEmptyHarvest, and it needs its own signal or that diagnostic
            // simply never runs.
            //
            // Shape only — window ids and package names, never titles or text.
            Log.w(
                TAG,
                "no root pkg=$packageName windows=${windows.size} " +
                    "pkgs=${windows.mapNotNull { it.root?.packageName?.toString() }.distinct()} " +
                    "activeRoot=${rootInActiveWindow?.packageName}",
            )
            return null
        }
        val root = resolved.root
        val texts = mutableListOf<String>()
        val resourceIds = mutableSetOf<String>()
        collectIdentity(root, texts, resourceIds, depth = 0, budget = NodeBudget())

        // Kept rather than removed after bring-up: this is how per-OEM
        // identifiers get collected, and the docs point contributors at it.
        // texts is a count, never the strings: the screen's contents stay on the
        // screen. It is here because an empty harvest on a visibly populated
        // screen is the signature of the node tree lagging the display, and
        // without it that looks identical to a screen that simply did not match.
        Log.d(
            TAG,
            "settings screen class=$className texts=${texts.size} " +
                "focused=${resolved.isFocused} ids=${resourceIds.take(12)}",
        )

        // A chrome-only harvest on a screen the user can plainly read is what
        // backlog item 1(b) was. It is closed, but a regression here is otherwise
        // silent, so the dump stays — on the failing case only, since it walks
        // the tree a second time.
        if (texts.size < DIAGNOSTIC_TEXT_FLOOR) logEmptyHarvest(packageName, root)

        return ScreenIdentity(
            packageName = packageName,
            className = className,
            windowFocused = resolved.isFocused,
            resourceIds = resourceIds,
            texts = texts,
        )
    }

    /**
     * Diagnostic for an empty harvest on a populated screen — backlog item 1(b).
     *
     * Kept after that item was closed, because it is the signal that says a
     * guarded screen is being harvested blind — a regression here is silent
     * otherwise. What it discriminates, corrected against what it measured:
     *
     *  - **Wrong window.** More than one window for [packageName] means [rootFor]
     *    picking the first match is a real suspect. Exactly one means it is not.
     *    This held: the device-admin list reported `windows=1`, which is what
     *    ruled item 2 out as a cause of 1(b) rather than merely ranking it last.
     *  - **Fetch failure.** `declared` counts children the tree says exist;
     *    `fetched` counts the ones `getChild` actually returned. A gap means
     *    nodes are being lost on the way out.
     *  - **Withheld subtree.** `declared == fetched` and still no text. Note this
     *    does *not* distinguish "absent" from "filtered", which is what the
     *    original version of this comment claimed: both `importantForAccessibility`
     *    filtering and `accessibilityDataSensitive` are applied by the framework
     *    before `childCount` is reported, so a withheld subtree shows no gap at
     *    all. 1(b) turned out to be exactly this case — `declared == fetched == 16`
     *    with the rows withheld — and was closed by `isAccessibilityTool`. Compare
     *    against a `uiautomator` dump to size the gap, remembering UiAutomation is
     *    exempt from both mechanisms.
     *
     * No text and no window titles are logged, only shape — the screen's contents
     * stay on the screen, same rule as the harvest log above.
     */
    private fun logEmptyHarvest(packageName: String, root: AccessibilityNodeInfo) {
        val matching = windows.filter { it.root?.packageName?.toString() == packageName }
        val shape = matching.joinToString { "id=${it.id} active=${it.isActive} focused=${it.isFocused} type=${it.type}" }

        var declared = 0
        var fetched = 0
        val budget = NodeBudget()
        fun walk(node: AccessibilityNodeInfo?, depth: Int) {
            if (node == null || depth > MAX_DEPTH) return
            if (!budget.take()) return
            declared += node.childCount
            for (i in 0 until node.childCount) {
                if (budget.exhausted) return
                val child = node.getChild(i) ?: continue
                fetched++
                walk(child, depth + 1)
            }
        }
        walk(root, depth = 0)

        // `truncated` matters for reading the other two numbers: a walk that ran
        // out of budget has a declared/fetched gap that means nothing.
        Log.w(
            TAG,
            "empty harvest pkg=$packageName windows=${matching.size} [$shape] " +
                "rootChildren=${root.childCount} declared=$declared fetched=$fetched " +
                "truncated=${budget.exhausted}",
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
        budget: NodeBudget,
    ) {
        if (node == null || depth > MAX_DEPTH || texts.size >= MAX_FRAGMENTS) return
        if (!budget.take()) return

        node.text?.toString()?.let { if (it.isNotBlank()) texts += it }
        node.contentDescription?.toString()?.let { if (it.isNotBlank()) texts += it }
        node.viewIdResourceName?.let { resourceIds += it }

        for (i in 0 until node.childCount) {
            collectIdentity(node.getChild(i), texts, resourceIds, depth + 1, budget)
            if (texts.size >= MAX_FRAGMENTS || budget.exhausted) return
        }
    }

    /**
     * Depth-first walk collecting `text` and `contentDescription`.
     *
     * Bounded by [MAX_DEPTH], [MAX_FRAGMENTS] and [MAX_NODES] because this runs
     * on the UI-event path and some apps (web views especially) expose very deep
     * *and* very wide trees; an unbounded walk here would jank the foreground app.
     */
    private fun collectText(
        node: AccessibilityNodeInfo?,
        into: MutableList<String>,
        depth: Int,
        budget: NodeBudget,
    ) {
        if (node == null || depth > MAX_DEPTH || into.size >= MAX_FRAGMENTS) return
        if (!budget.take()) return

        node.text?.toString()?.let { if (it.isNotBlank()) into += it }
        node.contentDescription?.toString()?.let { if (it.isNotBlank()) into += it }

        for (i in 0 until node.childCount) {
            collectText(node.getChild(i), into, depth + 1, budget)
            if (into.size >= MAX_FRAGMENTS || budget.exhausted) return
        }
    }

    /**
     * A per-walk allowance of node visits, shared down the recursion.
     *
     * Each visit costs an IPC round-trip to the app being inspected, so the
     * count that matters is across the whole walk rather than per level — which
     * is why this is threaded through rather than being a depth-local check.
     */
    private class NodeBudget(private var remaining: Int = MAX_NODES) {
        val exhausted: Boolean get() = remaining <= 0

        /** Claims one visit. False when the walk should stop. */
        fun take(): Boolean {
            if (remaining <= 0) return false
            remaining--
            return true
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

        /**
         * Ceiling on nodes visited per walk, across the whole tree.
         *
         * [MAX_DEPTH] bounds how *deep* a walk goes and [MAX_FRAGMENTS] bounds how
         * much text it keeps, but neither bounds how *wide* it goes: a node with
         * thousands of children stays under both while costing one IPC round-trip
         * per child on the UI-event path.
         *
         * The gap is not hypothetical here. [MAX_FRAGMENTS] only bites once text
         * has accumulated, so a tree that yields no text is bounded by depth
         * alone — and a tree that yields no text is precisely the screen the
         * empty-harvest diagnostic exists to investigate.
         *
         * Sized well above any real settings screen, so it is a backstop against
         * an ANR rather than a limit normal operation should ever reach.
         */
        private const val MAX_NODES = 3_000

        /**
         * Text-fragment count below which a settings harvest is treated as
         * suspect and [logEmptyHarvest] runs.
         *
         * Not zero, which is what this started as. The device-admin list harvests
         * *three* fragments — the toolbar title and its chrome — with none of the
         * list rows, so a strictly-empty trigger never fired on the one screen the
         * diagnostic was written for. Any real settings screen carries far more
         * than this, so the floor buys the chrome-only case without firing on
         * screens that are merely sparse.
         */
        private const val DIAGNOSTIC_TEXT_FLOOR = 8
    }
}
