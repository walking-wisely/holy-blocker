package com.holyblocker.mobile

import android.accessibilityservice.AccessibilityService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
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
import com.holyblocker.mobile.policy.UnwatchedEvent
import com.holyblocker.mobile.policy.WindowScreen

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
    private var protection: ProtectionStore? = null
    private var appliedGuardActive = false

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
        protection = ProtectionStore(this)

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
            // An event from an unwatched package is not by itself the user
            // leaving: SystemUI fires content-changed events for the status bar
            // drawn *over* whatever is in front. Measured on an android-36
            // emulator, one landed ~200 ms after the device-admin list opened
            // and, taken as a departure, cancelled that screen's entire re-look
            // budget — which is why the list then sat unguarded for as long as
            // it was open.
            when (guard.classifyUnwatchedEvent(foregroundPackage(), visiblePackages())) {
                // Chrome over a screen we are still guarding. Nothing to do,
                // and nothing to cancel.
                UnwatchedEvent.STILL_WATCHED -> return false

                // Tell the guard we left before skipping the harvest: the
                // re-fire suppression is only safe if it knows a back action
                // actually landed somewhere else, otherwise tapping straight
                // back into the settings screen falls inside the window and is
                // ignored.
                UnwatchedEvent.LEFT -> {
                    guard.onUnguardedScreen()
                    cancelRescan()
                }

                // Treat the bound as reset — the safe direction, since it only
                // ever makes the guard fire sooner — but leave the re-look
                // armed, because it is bounded and self-cancelling while an
                // unguarded screen is not.
                UnwatchedEvent.UNKNOWN -> guard.onUnguardedScreen()
            }
            return false
        }

        syncProtection(guard)

        val acted = applyDecision(guard, watchedWindows(guard, event), overlay)

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

        syncProtection(guard)

        // No event, so no class and no window id. The class is the unreliable
        // signal here anyway — resource ids and the self-mention catch-all both
        // still apply — but the missing window id does matter: focus is what
        // [watchedWindows] falls back on, and it is read from the window list
        // rather than from any event, so this path resolves windows exactly as
        // the event path does.
        return applyDecision(guard, watchedWindows(guard, event = null), overlay)
    }

    /**
     * Which app the user is looking at, for a re-look that has no event to ask.
     *
     * Falls back to the window list because on a settings task restored from
     * the background `rootInActiveWindow` is null — measured on an android-36
     * emulator, still null two seconds after the screen was fully drawn and
     * interactive — and that is exactly when the deferred look needs to work.
     */
    private fun foregroundPackage(): String? =
        rootInActiveWindow?.packageName?.toString()
            ?: windows.firstOrNull { it.isActive }?.root?.packageName?.toString()

    /**
     * Every package with a window on screen, focused or not.
     *
     * Separate from [foregroundPackage] because in split screen they genuinely
     * disagree, and the disagreement is the point — see the argument doc on
     * `SettingsGuard.classifyUnwatchedEvent`.
     */
    private fun visiblePackages(): List<String> =
        windows.mapNotNull { it.root?.packageName?.toString() }

    /**
     * Every visible window the guard watches, focused one first.
     *
     * Enumerating rather than reading a single root is what makes split screen
     * work. The previous version took the *first* window matching the event's
     * package, which in split screen is not necessarily the event's own window
     * and not necessarily the guarded one — so `mentionsSelf` was evaluated
     * against the wrong tree and the catch-all silently did nothing.
     *
     * Ordering matters twice over. [SettingsGuard] reads focus to choose
     * between backing out and covering, and the walk below shares one
     * [NodeBudget] across the whole pass — a per-window budget would multiply
     * the worst-case IPC cost on the UI-event path by the window count. Putting
     * the focused window first means the window the guard can actually act on
     * is the one that gets the allowance.
     *
     * `flagRetrieveInteractiveWindows` is already set in
     * `accessibility_service_config.xml`, so [windows] costs no new capability.
     */
    private fun watchedWindows(
        guard: SettingsGuard,
        event: AccessibilityEvent?,
    ): List<WindowScreen> {
        val budget = NodeBudget()

        val candidates = windows
            .mapNotNull { window -> window.root?.let { window to it } }
            .filter { (_, root) -> root.packageName?.toString()?.let(guard::watchesPackage) == true }
            .sortedByDescending { (window, _) -> window.isFocused }

        if (candidates.isNotEmpty()) {
            return candidates.map { (window, root) ->
                WindowScreen(
                    identity = identityOf(
                        root = root,
                        // The event's class describes the event's own window and
                        // nothing else. Attaching it to a sibling pane would
                        // hand that pane a class it does not have, and class is
                        // what several profile entries match on.
                        className = event?.className?.toString()
                            ?.takeIf { event.windowId == window.id },
                        window = window,
                        budget = budget,
                    ),
                    isFocused = window.isFocused,
                )
            }
        }

        // No usable window list. `rootInActiveWindow` is the only tree there is,
        // and a lone window is where BACK lands, so it is treated as focused —
        // which is exactly what the guard assumed before windows were resolved
        // at all.
        val root = rootInActiveWindow ?: return emptyList()
        val packageName = root.packageName?.toString() ?: return emptyList()
        if (!guard.watchesPackage(packageName)) return emptyList()

        return listOf(
            WindowScreen(
                identity = identityOf(
                    root = root,
                    className = event?.className?.toString(),
                    window = null,
                    budget = budget,
                ),
                isFocused = true,
            ),
        )
    }

    private fun identityOf(
        root: AccessibilityNodeInfo,
        className: String?,
        window: AccessibilityWindowInfo?,
        budget: NodeBudget,
    ): ScreenIdentity {
        val packageName = root.packageName?.toString().orEmpty()
        val texts = mutableListOf<String>()
        val resourceIds = mutableSetOf<String>()
        collectIdentity(root, texts, resourceIds, depth = 0, budget = budget)

        // Kept rather than removed after bring-up: this is how per-OEM
        // identifiers get collected, and the docs point contributors at it.
        // texts is a count, never the strings: the screen's contents stay on the
        // screen. It is here because an empty harvest on a visibly populated
        // screen is the signature of the node tree lagging the display, and
        // without it that looks identical to a screen that simply did not match.
        //
        // The window id and focus are logged because with more than one window
        // in play "which screen is this line about" is no longer obvious, and
        // focus is now what decides between backing out and covering.
        Log.d(
            TAG,
            "settings screen pkg=$packageName class=$className " +
                "window=${window?.id ?: -1} focused=${window?.isFocused ?: true} " +
                "texts=${texts.size} ids=${resourceIds.take(12)}",
        )

        // Kept after the empty harvest was diagnosed (see backlog.md, "Closed:
        // 1, the empty harvest"): the
        // cause was per-node accessibilityDataSensitive filtering, which is
        // invisible from this side — the nodes are simply not there — so the
        // shape of the tree is still the only evidence available when a screen
        // on an untested build comes back thin. Only on the failing case, since
        // it walks the tree a second time.
        if (texts.isEmpty()) logEmptyHarvest(packageName, root, starved = budget.exhausted)

        return ScreenIdentity(
            packageName = packageName,
            className = className,
            resourceIds = resourceIds,
            texts = texts,
        )
    }

    /**
     * Diagnostic for an empty harvest on a populated screen.
     *
     * This is what found the empty-harvest bug, and each number rules
     * something out:
     *
     *  - **Wrong window.** More than one window for [packageName] used to mean
     *    the guard could be reading the wrong tree. It read `windows=1` on the
     *    device-admin list, which is why that item and split screen turned out to
     *    be unrelated bugs. Every watched window is now evaluated, so this is a
     *    cross-check rather than a suspect — but it is still the number that says
     *    whether split screen is in play.
     *  - **Fetch failure.** `declared` counts children the tree says exist;
     *    `fetched` counts the ones `getChild` actually returned. A gap means
     *    children are failing to materialise. There was none: the two matched at
     *    every node.
     *  - **Genuinely absent.** `declared == fetched` with no text — what was in
     *    fact measured. The rows were withheld by `accessibilityDataSensitive`
     *    rather than lost anywhere in this walk, and the fix was
     *    `isAccessibilityTool` in `accessibility_service_config.xml`. Nodes
     *    filtered that way never appear in `childCount`, so a future instance of
     *    this reads as a subtree that is simply not there.
     *
     * No text and no window titles are logged, only shape — the screen's contents
     * stay on the screen, same rule as the harvest log above.
     */
    private fun logEmptyHarvest(
        packageName: String,
        root: AccessibilityNodeInfo,
        /**
         * Whether the pass ran out of node budget before reaching this window.
         *
         * The budget is shared across every window in a pass, so with split
         * screen an empty harvest now has a fourth possible cause that has
         * nothing to do with the tree: an earlier window spent the allowance.
         * Reading the numbers below without this would repeat the mistake the
         * whole diagnostic exists to prevent.
         */
        starved: Boolean,
    ) {
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
                "truncated=${budget.exhausted} starved=$starved",
        )
    }

    /** Applies a guard decision. Returns whether it acted. */
    private fun applyDecision(
        guard: SettingsGuard,
        screens: List<WindowScreen>,
        overlay: OverlayController,
    ): Boolean = when (val decision = guard.evaluate(screens, System.currentTimeMillis())) {
        is GuardDecision.BackOut -> {
            Log.i(TAG, "backing out of ${decision.surface}")
            // Clear first: leaving is the action, and a cover left behind
            // would sit over whatever screen we land on.
            overlay.apply(CoverState.CLEAR)
            // Safe to fire globally only because the guard returns BackOut for a
            // focused window and nothing else — this action takes no window
            // argument and lands wherever input focus is. See WindowScreen.
            performGlobalAction(GLOBAL_ACTION_BACK)
            true
        }

        is GuardDecision.CoverOnly -> {
            // Two different situations, both of which mean "do not press BACK":
            // the bound has been reached, so navigation is released before a
            // wrong matcher can make the device unusable; or the guarded screen
            // is an unfocused split-screen pane, where BACK would land on the
            // app the user is actually in.
            Log.w(TAG, "covering only on ${decision.surface}")
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

    /**
     * Applies the protection mode the user set in the app.
     *
     * Read on every watched event rather than cached: the mode changes from
     * `MainActivity`, and a disarm window also expires on its own with nothing
     * to announce it. `setGuardActive` is idempotent, so re-applying the same
     * answer costs nothing.
     */
    private fun syncProtection(guard: SettingsGuard) {
        val state = protection?.state() ?: return
        if (state.guardActive != appliedGuardActive) {
            appliedGuardActive = state.guardActive
            Log.i(TAG, "protection ${state.phase}; screen guard active=${state.guardActive}")
        }
        guard.setGuardActive(state.guardActive)
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
        protection = null
        appliedGuardActive = false
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
    }
}
