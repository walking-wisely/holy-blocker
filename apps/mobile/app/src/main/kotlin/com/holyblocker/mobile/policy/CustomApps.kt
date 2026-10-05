package com.holyblocker.mobile.policy

enum class AppRating { NEVER, ORDINARY }

enum class RefusalReason { INVALID_IDENTITY, PROTECTED, NEVER }

enum class RemovalPhase { NONE, PENDING, READY }

sealed interface AddOutcome {
    data class Added(val state: CustomAppState) : AddOutcome
    data class Refused(val reason: RefusalReason) : AddOutcome
}

data class CustomAppState(
    val apps: Set<String> = emptySet(),
    /** Package name to the `elapsedRealtime` at which its removal was requested. */
    val removals: Map<String, Long> = emptyMap(),
)

object AppRater {

    private val neverIdentities = setOf(
        "com.holyblocker.mobile",
        "com.android.systemui",
        "com.android.settings",
        "com.android.phone",
        "com.android.dialer",
        "com.android.emergency",
        "com.android.mms",
        "com.google.android.dialer",
        "com.google.android.apps.messaging",
        "com.google.android.apps.maps",
        "com.google.android.apps.authenticator2",
        "com.google.android.apps.walletnfcrel",
        "com.azure.authenticator",
        "com.authy.authy",
        "com.slack",
        "com.microsoft.teams",
        "us.zoom.videomeetings",
    )

    private val neverNameTokens = listOf(
        "bank", "wallet", "authenticator", "2fa", "emergency", "dialer", "medical", "health",
    )

    fun rate(identity: String): AppRating {
        val id = identity.trim().lowercase()
        if (id in neverIdentities) return AppRating.NEVER
        if (neverNameTokens.any { it in id }) return AppRating.NEVER
        if (payTokenPattern.containsMatchIn(id)) return AppRating.NEVER
        return AppRating.ORDINARY
    }

    private val payTokenPattern = Regex("""(^|[._])(pay|payment|payments|paypal)([._]|$)|paypal|googlepay""")
}

object CustomApps {

    private val identityPattern = Regex("""[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+""")

    fun add(state: CustomAppState, identity: String, protected: Set<String>): AddOutcome {
        val id = identity.trim()
        if (!identityPattern.matches(id)) return AddOutcome.Refused(RefusalReason.INVALID_IDENTITY)
        if (id in protected) return AddOutcome.Refused(RefusalReason.PROTECTED)
        if (AppRater.rate(id) == AppRating.NEVER) return AddOutcome.Refused(RefusalReason.NEVER)
        return AddOutcome.Added(CustomAppState(state.apps + id, state.removals - id))
    }

    fun requestRemoval(state: CustomAppState, identity: String, nowElapsed: Long): CustomAppState {
        if (identity !in state.apps) return state
        if (removalPhase(state.removals[identity], nowElapsed) != RemovalPhase.NONE) return state
        return state.copy(removals = state.removals + (identity to nowElapsed))
    }

    fun cancelRemoval(state: CustomAppState, identity: String): CustomAppState =
        state.copy(removals = state.removals - identity)

    fun removalPhase(requestedAt: Long?, nowElapsed: Long): RemovalPhase {
        if (requestedAt == null || nowElapsed < requestedAt) return RemovalPhase.NONE
        val progress = nowElapsed - requestedAt
        return when {
            progress < ProtectionSchedule.COOLDOWN_MILLIS -> RemovalPhase.PENDING
            progress < ProtectionSchedule.COOLDOWN_MILLIS + ProtectionSchedule.READY_WINDOW_MILLIS ->
                RemovalPhase.READY
            else -> RemovalPhase.NONE
        }
    }

    fun confirmRemoval(state: CustomAppState, identity: String, nowElapsed: Long): CustomAppState? {
        if (identity !in state.apps) return null
        if (removalPhase(state.removals[identity], nowElapsed) != RemovalPhase.READY) return null
        return CustomAppState(state.apps - identity, state.removals - identity)
    }

    fun prune(state: CustomAppState, nowElapsed: Long): CustomAppState = state.copy(
        removals = state.removals.filter { (id, at) ->
            id in state.apps && removalPhase(at, nowElapsed) != RemovalPhase.NONE
        },
    )

    fun shouldEnforce(
        state: CustomAppState,
        identity: String,
        protected: Set<String>,
        protection: ProtectionState,
    ): Boolean = protection.guardActive &&
        identity in state.apps &&
        identity !in protected &&
        AppRater.rate(identity) != AppRating.NEVER

    private const val MIN_TOKEN_LENGTH = 5

    private val leadingDomainSegments =
        setOf("com", "org", "net", "io", "co", "app", "me", "ua", "ru", "de", "tv")

    private val genericSegments = setOf("android", "app", "apps", "mobile", "lite", "beta", "client", "google")

    private val stoplist = setOf(
        "music", "games", "photos", "video", "videos", "store", "shop", "browser", "camera", "gallery",
        "calendar", "contacts", "clock", "files", "notes", "weather", "search", "messenger", "social",
        "news", "radio", "market", "travel", "fitness", "network", "system", "service", "services",
    )

    fun tokens(identity: String, displayName: String?): Set<String> {
        val distinctive = identity.trim().lowercase()
            .split('.')
            .dropWhile { it in leadingDomainSegments }
            .firstOrNull { it !in genericSegments }
        return listOfNotNull(distinctive, displayName)
            .map { reduce(it) }
            .filter { it.length >= MIN_TOKEN_LENGTH && it !in stoplist }
            .toSet()
    }

    private fun reduce(text: String): String =
        text.lowercase().filter { it in 'a'..'z' || it in '0'..'9' }
}
