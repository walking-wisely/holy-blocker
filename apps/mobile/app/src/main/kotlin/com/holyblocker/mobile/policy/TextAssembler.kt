package com.holyblocker.mobile.policy

/**
 * Flattens the text fragments harvested from an accessibility node tree into a
 * single string for the policy engine.
 *
 * Kept separate from the service so the traversal glue (which needs real
 * `AccessibilityNodeInfo` objects) stays thin and this part stays testable.
 */
object TextAssembler {
    /** Fragments beyond this are dropped: a scan runs on the UI-event path. */
    const val DEFAULT_MAX_CHARS: Int = 4_000

    /**
     * Joins [fragments] with single spaces, collapsing internal whitespace and
     * dropping blanks. Returns null when nothing usable remains, which the gate
     * treats as "nothing to scan".
     *
     * Truncates to [maxChars] on a whitespace boundary where possible so the
     * final token is not cut mid-word — a half-word would defeat the lexicon's
     * token-boundary matching rather than merely shorten the input.
     */
    fun assemble(fragments: List<String>, maxChars: Int = DEFAULT_MAX_CHARS): String? {
        if (maxChars <= 0) return null

        val joined = fragments
            .asSequence()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .joinToString(" ")
            .replace(WHITESPACE_RUN, " ")
            .trim()

        if (joined.isEmpty()) return null
        if (joined.length <= maxChars) return joined

        val hard = joined.substring(0, maxChars)
        val lastSpace = hard.lastIndexOf(' ')
        val cut = if (lastSpace > 0) hard.substring(0, lastSpace) else hard
        return cut.trim().ifEmpty { null }
    }

    private val WHITESPACE_RUN = Regex("\\s+")
}
