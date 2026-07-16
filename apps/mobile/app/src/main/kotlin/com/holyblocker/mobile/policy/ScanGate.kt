package com.holyblocker.mobile.policy

/** What the overlay should do as a result of a scan. */
enum class CoverState {
    COVER,
    WARN,
    CLEAR,
}

/** Why a screen update did not reach the policy engine. */
enum class SkipReason {
    /** Our own overlay is on screen; scanning it would feed us our own text. */
    SELF_PACKAGE,

    /** Nothing usable came out of the node tree. */
    NO_TEXT,

    /** Identical text to the last scan in this app — the verdict cannot differ. */
    DUPLICATE,

    /** Too soon after the last scan of this same app. */
    DEBOUNCED,
}

sealed interface GateOutcome {
    data class Scanned(val verdict: PolicyVerdict, val cover: CoverState) : GateOutcome

    data class Skipped(val reason: SkipReason) : GateOutcome
}

/**
 * Decides which accessibility updates are worth a policy evaluation, and turns
 * the verdict into an overlay state.
 *
 * `AccessibilityService` fires window-state and scroll events far faster than a
 * human scrolls — several per frame while a list is moving — and every one of
 * them would otherwise mean a full normalize + lexicon pass. Evaluation is
 * cheap but not free, and it runs on the UI-event path, so this gate exists to
 * drop the redundant ones.
 *
 * Not thread-safe: the accessibility callback is single-threaded, and this is
 * built to be called from it.
 */
class ScanGate(
    private val policy: TextPolicy,
    private val selfPackage: String,
    private val minRescanIntervalMillis: Long = DEFAULT_MIN_RESCAN_INTERVAL_MILLIS,
    private val maxTextChars: Int = TextAssembler.DEFAULT_MAX_CHARS,
) {
    private var lastPackage: String? = null
    private var lastText: String? = null
    private var lastScanAtMillis: Long = 0

    /**
     * Cheap pre-check letting the caller skip harvesting entirely.
     *
     * Walking the node tree is the expensive part and it happens on the
     * UI-event path, so the service asks this before collecting anything. The
     * rule still lives here (and is enforced again in [onScreenText]) rather
     * than in the service, so it stays tested.
     */
    fun shouldHarvest(packageName: String): Boolean = packageName != selfPackage

    fun onScreenText(
        packageName: String,
        fragments: List<String>,
        nowMillis: Long,
    ): GateOutcome {
        if (!shouldHarvest(packageName)) {
            return GateOutcome.Skipped(SkipReason.SELF_PACKAGE)
        }

        val text = TextAssembler.assemble(fragments, maxTextChars)
            ?: return GateOutcome.Skipped(SkipReason.NO_TEXT)

        // Switching apps is the highest-signal moment there is: the whole screen
        // just changed. Never debounce or dedupe across that boundary.
        val switchedApp = packageName != lastPackage
        if (!switchedApp) {
            if (text == lastText) {
                return GateOutcome.Skipped(SkipReason.DUPLICATE)
            }
            if (nowMillis - lastScanAtMillis < minRescanIntervalMillis) {
                return GateOutcome.Skipped(SkipReason.DEBOUNCED)
            }
        }

        val verdict = policy.evaluate(text, PolicySource.ACCESSIBILITY_TREE)

        lastPackage = packageName
        lastText = text
        lastScanAtMillis = nowMillis

        return GateOutcome.Scanned(verdict, coverFor(verdict.action))
    }

    /** Drops memoised state, e.g. when the service reconnects. */
    fun reset() {
        lastPackage = null
        lastText = null
        lastScanAtMillis = 0
    }

    companion object {
        /**
         * Chosen so a fast scroll costs a bounded number of scans per second
         * while a deliberate screen change still feels immediate.
         */
        const val DEFAULT_MIN_RESCAN_INTERVAL_MILLIS: Long = 300

        /**
         * BLUR maps to COVER for now: the MVP overlay is opaque and has no
         * partial-obscure mode, and per the mission a tool that almost blocks is
         * not the same as one that blocks. Erring toward covering matches the
         * formation model's "tune blocking for recall".
         */
        fun coverFor(action: PolicyAction): CoverState = when (action) {
            PolicyAction.BLOCK, PolicyAction.BLUR -> CoverState.COVER
            PolicyAction.WARN -> CoverState.WARN
            PolicyAction.LOG, PolicyAction.ALLOW -> CoverState.CLEAR
        }
    }
}
