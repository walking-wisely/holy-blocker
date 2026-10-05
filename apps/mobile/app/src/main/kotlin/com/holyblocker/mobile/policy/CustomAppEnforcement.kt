package com.holyblocker.mobile.policy

data class EnforcementAction(val cover: Boolean, val sendHome: Boolean)

class CustomAppEnforcement(private val minGapMillis: Long = DEFAULT_MIN_GAP_MILLIS) {

    private var lastHomeAt: Long? = null

    fun decide(
        state: CustomAppState,
        visiblePackages: Collection<String>,
        protected: Set<String>,
        protection: ProtectionState,
        nowElapsed: Long,
    ): EnforcementAction {
        val listedVisible = visiblePackages.any {
            CustomApps.shouldEnforce(state, it, protected, protection)
        }
        if (!listedVisible) return EnforcementAction(cover = false, sendHome = false)

        val last = lastHomeAt
        val due = last == null || nowElapsed < last || nowElapsed - last >= minGapMillis
        if (due) lastHomeAt = nowElapsed
        return EnforcementAction(cover = true, sendHome = due)
    }

    private companion object {
        const val DEFAULT_MIN_GAP_MILLIS = 1_000L
    }
}
