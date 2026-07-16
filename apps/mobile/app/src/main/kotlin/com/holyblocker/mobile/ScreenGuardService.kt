package com.holyblocker.mobile

import android.accessibilityservice.AccessibilityService
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.holyblocker.mobile.policy.GateOutcome
import com.holyblocker.mobile.policy.NativeTextPolicy
import com.holyblocker.mobile.policy.ScanGate

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

    override fun onServiceConnected() {
        super.onServiceConnected()
        val engine = NativeTextPolicy.withBuiltinDictionary()
        policy = engine
        gate = ScanGate(policy = engine, selfPackage = packageName)
        overlay = OverlayController(this)
        Log.i(TAG, "screen guard connected")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        val gate = gate ?: return
        val overlay = overlay ?: return
        val packageName = event?.packageName?.toString() ?: return

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
    }

    companion object {
        private const val TAG = "ScreenGuard"
        private const val MAX_DEPTH = 40
        private const val MAX_FRAGMENTS = 400
    }
}
