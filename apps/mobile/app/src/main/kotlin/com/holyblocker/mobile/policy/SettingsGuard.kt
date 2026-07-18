package com.holyblocker.mobile.policy

/**
 * A window, described without Android types so matching stays testable on the JVM.
 *
 * [resourceIds] are `getViewIdResourceName()` values from the node tree — e.g.
 * `com.android.settings:id/recycler_view`. They require `flagReportViewIds`,
 * which `accessibility_service_config.xml` already sets.
 */
data class ScreenIdentity(
    val packageName: String,
    val className: String?,
    val resourceIds: Set<String> = emptySet(),
    val texts: List<String> = emptyList(),
    /**
     * Whether the window this was harvested from holds input focus.
     *
     * Load-bearing for the decision, not descriptive: `GLOBAL_ACTION_BACK` takes
     * no window argument and lands on whatever is focused, so backing out of an
     * unfocused match would press BACK inside the app the user is actually
     * using. Defaults true because the single-window case — every screen outside
     * split screen — is exactly that.
     */
    val windowFocused: Boolean = true,
)

/** A screen whose purpose is removing the guard. */
enum class GuardedSurface {
    /** Settings → Accessibility: the disable toggle. */
    ACCESSIBILITY_SETTINGS,

    /** Device admin activation/deactivation, which gates uninstall. */
    DEVICE_ADMIN_SETTINGS,

    /** App Info for our own package: force-stop, clear data, uninstall. */
    APP_INFO_SELF,

    /**
     * The system uninstall confirmation for our own package.
     *
     * Reached by long-pressing the launcher icon, which never touches the
     * settings app — so none of the settings identifiers apply and this needs
     * its own watched package. Three taps from the home screen, and pure muscle
     * memory rather than anything a user has to look up.
     */
    UNINSTALL_SELF,

    /**
     * A settings screen that names this app but did not match a known activity.
     *
     * The catch-all, and load-bearing rather than defensive: the activity class
     * arrives only on `TYPE_WINDOW_STATE_CHANGED`, and that event is **not
     * reliably delivered** — opening the accessibility list can produce nothing
     * but content-changed events carrying `FrameLayout`. Class matching alone
     * therefore leaves the screen unguarded some of the time.
     *
     * Screens in the settings app that name us are all removal-adjacent: the
     * accessibility list, our own service page, our App Info, our notification
     * and battery pages. Guarding the set is deliberate over-reach with a known
     * shape, and the back-out bound limits what a false positive costs.
     */
    SELF_IN_SETTINGS,
}

sealed interface GuardDecision {
    /** Not a guarded screen, or guarding is suspended. */
    data object Ignore : GuardDecision

    /** Leave the screen before the control is reachable. */
    data class BackOut(val surface: GuardedSurface) : GuardDecision

    /**
     * Backing out is not working. Navigation is released and only the cover
     * remains, so a wrong matcher cannot make the device unusable.
     */
    data class CoverOnly(val surface: GuardedSurface) : GuardDecision
}

/**
 * One activity that hosts a guarded screen.
 *
 * [requiresSelfMention] marks the generic host activities. Settings reuses a
 * handful of container activities for most of its pages, so the class alone says
 * "this is a Settings sub-page", not *which* one. Requiring the screen to name
 * us narrows those to the pages that are about this app — and our own label is a
 * brand string, so unlike Settings' own copy it does not change with the device
 * language.
 */
data class GuardedScreen(
    val className: String,
    val surface: GuardedSurface,
    val requiresSelfMention: Boolean = false,
)

/**
 * Per-OEM identifiers for the screens that remove the guard.
 *
 * Deliberately data rather than code: vendors ship their own Settings app, so
 * extending coverage means adding a table entry with test cases, not editing
 * matching logic.
 *
 * **Every entry must be dumped from a running device, never inferred.** The
 * first draft of this table was written from plausible-looking class names and
 * almost all of them were wrong — the real App Info activity is a generic
 * Compose host, device admin lives under a longer package path, and the
 * accessibility screens have no distinctive resource ids whatsoever. Use
 * `adb shell dumpsys activity activities | grep topResumedActivity` on each
 * screen, and the `settings screen class=…` debug log the service emits.
 */
data class SettingsProfile(
    val name: String,
    val settingsPackages: Set<String>,
    val screens: List<GuardedScreen>,
    /**
     * Packages hosting the system uninstall confirmation.
     *
     * Matched on self-mention only, never by class: this dialog is shared by
     * every uninstall on the device, and blocking anything but our own would
     * stop the user managing their own phone.
     */
    val uninstallerPackages: Set<String> = DEFAULT_UNINSTALLER_PACKAGES,
    /**
     * Resource ids that identify a guarded screen outright.
     *
     * Empty on AOSP: its accessibility pages expose only generic Settings
     * chrome (`recycler_view`, `content_frame`, `app_bar`) shared by every
     * sub-page. Kept because a vendor with distinctive ids would be matched more
     * cheaply and precisely this way than by class name.
     */
    val resourceIdSurfaces: Map<String, GuardedSurface> = emptyMap(),
) {
    companion object {
        /**
         * `com.google.android.packageinstaller` is what the android-36 emulator
         * reports; the AOSP name is kept for builds without Google's variant.
         */
        val DEFAULT_UNINSTALLER_PACKAGES = setOf(
            "com.google.android.packageinstaller",
            "com.android.packageinstaller",
        )
    }
}

object SettingsProfiles {
    /**
     * AOSP / Pixel, verified against the `android-36 google_apis arm64-v8a`
     * emulator image that `scripts/smoke-test.sh` targets.
     */
    val AOSP = SettingsProfile(
        name = "aosp",
        settingsPackages = setOf("com.android.settings"),
        screens = listOf(
            // The accessibility list. Dedicated activity, verified on device as
            // the class the window-state event actually reports.
            GuardedScreen(
                "com.android.settings.Settings\$AccessibilitySettingsActivity",
                GuardedSurface.ACCESSIBILITY_SETTINGS,
            ),
            // The shortcut detail page. Present in the package manager's
            // activity list but not exercised, so treat as unconfirmed.
            GuardedScreen(
                "com.android.settings.Settings\$AccessibilityDetailsSettingsActivity",
                GuardedSurface.ACCESSIBILITY_SETTINGS,
            ),
            // The per-service page — the one carrying our actual on/off switch —
            // is the generic SubSettings host, so it only counts when it is
            // showing us.
            GuardedScreen(
                "com.android.settings.SubSettings",
                GuardedSurface.ACCESSIBILITY_SETTINGS,
                requiresSelfMention = true,
            ),
            // Device admin: the activation dialog. The class exists on this
            // image, but the screen could not be opened for verification — it
            // requires EXTRA_DEVICE_ADMIN naming a DeviceAdminReceiver, and we
            // do not have one yet. Confirm the reported event class when the
            // receiver lands rather than trusting this entry.
            //
            // Settings$DeviceAdminSettingsActivity is deliberately absent: it is
            // an alias that resolves to com.android.settings/.Settings, so the
            // event would carry the main Settings class and matching on that
            // would eject the user from all of Settings.
            GuardedScreen(
                "com.android.settings.applications.specialaccess.deviceadmin.DeviceAdminAdd",
                GuardedSurface.DEVICE_ADMIN_SETTINGS,
            ),
            // App Info is rendered by Settings' Compose SPA host, which is as
            // generic as SubSettings and needs the same narrowing.
            GuardedScreen(
                "com.android.settings.spa.SpaActivity",
                GuardedSurface.APP_INFO_SELF,
                requiresSelfMention = true,
            ),
        ),
        // DeviceAdminAdd's own view ids, dumped from the emulator. Unlike every
        // other screen in this profile, this one cannot be matched by class:
        // opening it emits events carrying android.widget.FrameLayout, and the
        // activity class arrives only on a later event that is not always sent.
        //
        // Matching by id matters more here than anywhere else, because this is
        // the one guarded screen that is sometimes *exempt* — see isDeviceAdmin-
        // Active in SettingsGuard. An exemption that fails to identify the
        // screen does not fail open, it fails into the self-mention catch-all
        // and blocks activation entirely.
        resourceIdSurfaces = mapOf(
            "com.android.settings:id/admin_name" to GuardedSurface.DEVICE_ADMIN_SETTINGS,
            "com.android.settings:id/add_msg" to GuardedSurface.DEVICE_ADMIN_SETTINGS,
            "com.android.settings:id/admin_warning" to GuardedSurface.DEVICE_ADMIN_SETTINGS,
        ),
    )

    /**
     * Resolves the profile for a device, or null when it is untested.
     *
     * The emulator reports `Google`; `unknown` covers plain AOSP builds.
     * Matching is case-insensitive because `Build.MANUFACTURER` casing is not
     * consistent across builds.
     */
    fun forManufacturer(manufacturer: String): SettingsProfile? =
        when (manufacturer.lowercase()) {
            "google", "unknown" -> AOSP
            else -> null
        }
}

/**
 * Blocks the screens that would remove the guard.
 *
 * At plain Device Admin there is no policy API that prevents the accessibility
 * service being disabled, the app being force-stopped, or the admin being
 * deactivated. What is available is that the service is still running while the
 * user is on the screen that would do it — so the screen is identified and left
 * before the control is reachable.
 *
 * Backing out rather than covering is deliberate: it removes the race between
 * the window rendering and an overlay attaching, which a user who knows where
 * the toggle sits would otherwise win.
 *
 * This is friction, not prevention. Safe mode, `adb` and factory reset all
 * remain and cannot be observed from here. That ceiling is intended — the goal
 * is that removal costs deliberate effort rather than a reflex.
 *
 * Not thread-safe: the accessibility callback is single-threaded, and this is
 * built to be called from it.
 */
class SettingsGuard(
    private val profile: SettingsProfile?,
    private val selfPackage: String,
    private val selfLabel: String,
    /**
     * Whether our `DeviceAdminReceiver` is currently active.
     *
     * A supplier rather than a value because the guard is built once in
     * `onServiceConnected` and the admin is normally enabled later, from
     * onboarding. See [match] for what it gates.
     */
    private val isDeviceAdminActive: () -> Boolean,
) {
    private var lastSurface: GuardedSurface? = null
    private var consecutiveBackOuts = 0
    private var lastBackOutAtMillis = 0L
    private var suspendedUntilMillis = 0L

    /** False when the device has no verified identifiers; the UI must say so. */
    val isDeviceSupported: Boolean get() = profile != null

    fun isSuspended(nowMillis: Long): Boolean = nowMillis < suspendedUntilMillis

    /**
     * Cheap pre-check letting the caller skip harvesting entirely.
     *
     * [evaluate] needs resource ids, which means a pass over the node tree on
     * the UI-event path. Only the settings app can host a guarded screen, so
     * everything else is rejected before that cost is paid.
     */
    fun watchesPackage(packageName: String): Boolean = profile != null &&
        (packageName in profile.settingsPackages || packageName in profile.uninstallerPackages)

    /**
     * Releases the guard until [untilMillis].
     *
     * The exit path is required, not optional. Until the per-OEM identifiers are
     * verified on real hardware, a matcher that is wrong on an untested build
     * would otherwise leave the user locked out of their own settings with no
     * in-app recovery.
     */
    fun suspendUntil(untilMillis: Long) {
        suspendedUntilMillis = untilMillis
        resetBound()
    }

    /**
     * Tells the guard the user is somewhere that is not the settings app.
     *
     * Must be called for every screen [watchesPackage] rejects, cheap as it is,
     * because the re-fire suppression below is only safe if the guard knows when
     * the user has actually left. Without it, backing out and tapping straight
     * back in lands inside the suppression window and the guard sits idle — and
     * since [evaluate] only runs when an event fires, a static screen that has
     * finished rendering never wakes it again. The toggle would stay reachable
     * for as long as the user cared to leave it open.
     */
    fun onUnguardedScreen() {
        resetBound()
    }

    fun evaluate(screen: ScreenIdentity, nowMillis: Long): GuardDecision {
        if (profile == null || isSuspended(nowMillis)) {
            return GuardDecision.Ignore
        }

        val surface = match(screen)
        if (surface == null) {
            // The user left, so any loop we were in has ended.
            resetBound()
            return GuardDecision.Ignore
        }

        // A guarded screen we cannot back out of, because the back action would
        // land somewhere else entirely.
        //
        // `GLOBAL_ACTION_BACK` takes no window argument; it goes to the focused
        // window. In split screen the matched settings pane need not be the
        // focused one, and firing BACK then presses it inside whatever app the
        // user is actually driving while never dismissing the pane that matched.
        // Left to run, that repeats until the bound trips — roughly 3.6s of stray
        // BACK presses in an innocent app, ending in a cover anyway.
        //
        // Deliberately before the bookkeeping below: covering is not an attempt
        // at leaving, so it must not consume the back-out budget. A pane parked
        // unfocused in split screen would otherwise exhaust the budget without a
        // single BACK being sent, leaving the guard degraded for the moment the
        // user finally focuses it.
        if (!screen.windowFocused) {
            return GuardDecision.CoverOnly(surface)
        }

        val sinceLast = nowMillis - lastBackOutAtMillis

        // A screen emits several window-state events while it renders — three in
        // ~800 ms for the accessibility list on AOSP. Firing BACK on each would
        // pop several levels of the navigation stack instead of one, and would
        // spend the whole loop budget on a single visit. Wait for the back we
        // already sent to land before deciding it did not work.
        if (surface == lastSurface && sinceLast < BACK_OUT_REFIRE_MILLIS) {
            return GuardDecision.Ignore
        }

        val continuing = surface == lastSurface && sinceLast < BACK_OUT_RESET_MILLIS

        consecutiveBackOuts = if (continuing) consecutiveBackOuts + 1 else 1
        lastSurface = surface
        lastBackOutAtMillis = nowMillis

        return if (consecutiveBackOuts > MAX_CONSECUTIVE_BACK_OUTS) {
            GuardDecision.CoverOnly(surface)
        } else {
            GuardDecision.BackOut(surface)
        }
    }

    /** Drops memoised state, e.g. when the service reconnects. */
    fun reset() {
        resetBound()
        suspendedUntilMillis = 0
    }

    private fun resetBound() {
        lastSurface = null
        consecutiveBackOuts = 0
        lastBackOutAtMillis = 0
    }

    private fun match(screen: ScreenIdentity): GuardedSurface? {
        if (profile == null) return null

        // The uninstall dialog is shared with every other app on the device, so
        // it is guarded only when it names us — never by class.
        if (screen.packageName in profile.uninstallerPackages) {
            return if (mentionsSelf(screen)) GuardedSurface.UNINSTALL_SELF else null
        }

        if (screen.packageName !in profile.settingsPackages) return null

        // A resource id, where a vendor has a distinctive one, is the cheapest
        // and most precise signal — and unlike Settings' visible copy it does
        // not change with the device language.
        screen.resourceIds.firstNotNullOfOrNull { profile.resourceIdSurfaces[it] }
            ?.let { return narrow(it, screen) }

        profile.screens
            .firstOrNull { it.className == screen.className && (!it.requiresSelfMention || mentionsSelf(screen)) }
            ?.let { return narrow(it.surface, screen) }

        // Last resort, and the one that actually holds: the activity class is
        // only on window-state events and those are not always delivered, so a
        // screen naming us is treated as guarded whatever class it reports.
        return if (mentionsSelf(screen)) GuardedSurface.SELF_IN_SETTINGS else null
    }

    /**
     * Applies the rules that hold however the surface was identified.
     *
     * Both are about the same activity and both must survive a match by
     * resource id as well as by class, which is why they live here rather than
     * on `GuardedScreen`.
     */
    private fun narrow(surface: GuardedSurface, screen: ScreenIdentity): GuardedSurface? {
        // DeviceAdminAdd hosts the prompt for *every* device admin on the
        // phone, exactly like the uninstall dialog. Guarding it without
        // checking whose admin it is would eject the user from managing
        // somebody else's app, which is well outside what this tool may do.
        if (surface in SELF_MENTION_REQUIRED && !mentionsSelf(screen)) return null

        return exempt(surface)
    }

    /**
     * Drops the one surface that is conditional.
     *
     * `DeviceAdminAdd` is a single activity serving both directions: it is the
     * activation prompt while the admin is off and the deactivation prompt once
     * it is on. Guarding it unconditionally would eject the user from the only
     * screen that can enable the admin, so the feature could never be turned on.
     *
     * Applied to *identified* surfaces only, and returning null rather than
     * letting the caller fall through, because this screen names the app: the
     * self-mention catch-all would otherwise re-guard it as `SELF_IN_SETTINGS`
     * and the exemption would be silently dead. That is not a hypothetical — it
     * is what the first version of this did on a real device.
     */
    private fun exempt(surface: GuardedSurface): GuardedSurface? =
        if (surface == GuardedSurface.DEVICE_ADMIN_SETTINGS && !isDeviceAdminActive()) {
            null
        } else {
            surface
        }

    private fun mentionsSelf(screen: ScreenIdentity): Boolean =
        screen.texts.any { it.contains(selfLabel, ignoreCase = true) || it.contains(selfPackage) }

    companion object {
        /**
         * How many times we will back out of the same screen before releasing
         * navigation. Low on purpose: the cost of being wrong is a user who
         * cannot reach Settings at all, and three attempts is already well past
         * what a working back action needs.
         */
        const val MAX_CONSECUTIVE_BACK_OUTS = 3

        /**
         * How long a fired back action is given to take effect before the same
         * screen counts as a second attempt.
         *
         * Sized from the render burst measured on device (three window-state
         * events across ~800 ms) with headroom, so one visit costs one attempt.
         */
        const val BACK_OUT_REFIRE_MILLIS = 1_200L

        /**
         * Quiet period after which a return to the same screen counts as a fresh
         * attempt rather than a continuation of a loop that already ended.
         */
        const val BACK_OUT_RESET_MILLIS = 5_000L

        /**
         * Surfaces whose activity is shared with other apps, so a match only
         * counts when the screen names us.
         *
         * `UNINSTALL_SELF` is not listed because it is already reached by a
         * self-mention-only path in [match] — the installer package never
         * matches by class at all.
         */
        private val SELF_MENTION_REQUIRED = setOf(GuardedSurface.DEVICE_ADMIN_SETTINGS)
    }
}
