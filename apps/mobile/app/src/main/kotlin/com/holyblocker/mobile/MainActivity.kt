package com.holyblocker.mobile

import android.Manifest
import android.app.Activity
import android.app.admin.DevicePolicyManager
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import com.holyblocker.mobile.admin.HolyBlockerAdminReceiver
import com.holyblocker.mobile.policy.AccessibilityServiceStatus
import com.holyblocker.mobile.policy.NetworkGuard
import com.holyblocker.mobile.policy.ProtectionPhase
import com.holyblocker.mobile.policy.ProtectionState
import com.holyblocker.mobile.policy.ScreenCapture
import com.holyblocker.mobile.policy.SettingsProfiles

/**
 * Onboarding surface. The MVP has one job: get the user to the accessibility
 * toggle and explain the Restricted Settings detour that a sideloaded app hits
 * on Android 13+ (see docs/decisions/content-interception.md — sideload friction
 * is intentional here, and the detour is authenticated by the device PIN).
 */
class MainActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var coverage: TextView
    private lateinit var open: Button
    private lateinit var protectionAction: Button
    private lateinit var cancel: Button
    private lateinit var removalHint: TextView
    private lateinit var adminStatus: TextView
    private lateinit var admin: Button
    private lateinit var networkStatus: TextView
    private lateinit var network: Button
    private lateinit var networkHint: TextView
    private lateinit var captureStatus: TextView
    private lateinit var capture: Button
    private lateinit var captureHint: TextView
    private lateinit var protection: ProtectionStore

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        protection = ProtectionStore(this)
        status = TextView(this).apply { textSize = 18f }

        coverage = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        // One button whose meaning follows the phase — arm, request, confirm —
        // so there is never a live "turn it off now" control sitting beside an
        // armed guard. Its listener is rebound in refresh().
        protectionAction = Button(this)

        // Separate from the action button on purpose: abandoning a request must
        // not be one mis-tap away from confirming a disarm.
        cancel = Button(this).apply {
            text = getString(R.string.action_cancel_disarm)
            setOnClickListener {
                protection.cancelDisarm()
                refresh()
            }
        }

        removalHint = TextView(this).apply {
            text = getString(R.string.hint_disarm_to_remove)
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        adminStatus = TextView(this).apply { textSize = 18f }

        networkStatus = TextView(this).apply { textSize = 18f }

        networkHint = TextView(this).apply {
            text = getString(R.string.network_guard_explanation)
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        // Only ever offers to turn it on, for the same reason as the admin
        // button: the way back out is the system VPN screen, which the timed
        // release exists to reach. A "turn off filtering" button here would be
        // a one-tap bypass of the release flow.
        network = Button(this).apply {
            text = getString(R.string.action_enable_network_guard)
            // startActivityForResult, deprecated in AndroidX but the only API
            // available to a plain Activity — this module has no AndroidX
            // dependency. The result is not read: onResume re-reads the grant
            // from VpnService.prepare(), which is the authority either way, and
            // is also what catches a revoke that happened elsewhere.
            setOnClickListener {
                VpnService.prepare(this@MainActivity)?.let {
                    startActivityForResult(it, REQUEST_VPN_CONSENT)
                }
            }
        }

        captureStatus = TextView(this).apply { textSize = 18f }

        captureHint = TextView(this).apply {
            text = getString(R.string.capture_explanation)
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        // Unlike the VPN button this one comes back: a MediaProjection grant is
        // per session and cannot be stored, so every restart — and every tap of
        // the system's stop-sharing chip — puts this back on screen. That is
        // Android's design and it is stated in the copy rather than hidden, since
        // a user who believes scanning survived a reboot is worse off than one
        // who knows it did not.
        capture = Button(this).apply {
            text = getString(R.string.action_enable_capture)
            setOnClickListener {
                ScreenCaptureService.captureIntent(this@MainActivity)?.let {
                    startActivityForResult(it, REQUEST_CAPTURE_CONSENT)
                }
            }
        }

        // Only ever offers activation. Deactivation is deliberately not mirrored
        // here: it belongs to the system screen, which is guarded like every
        // other removal path and reachable through the timed release. A
        // deactivate button beside it would be the same one-tap bypass the
        // release flow was reshaped to remove.
        admin = Button(this).apply {
            text = getString(R.string.action_enable_admin)
            setOnClickListener { startActivity(addAdminIntent()) }
        }

        val hint = TextView(this).apply {
            text = getString(R.string.restricted_settings_hint)
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        // Shown only while the guard is not yet running, or during an open
        // release window. With the guard active this is a one-tap route to the
        // toggle that turns it off, sitting directly beneath the release button
        // — which is what made the exit path faster than every bypass it was
        // built to close.
        open = Button(this).apply {
            text = getString(R.string.action_open_accessibility_settings)
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            }
        }

        setContentView(
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(48, 48, 48, 48)
                addView(TextView(context).apply {
                    text = getString(R.string.onboarding_title)
                    textSize = 28f
                })
                addView(status)
                addView(coverage)
                addView(open)
                addView(protectionAction)
                addView(cancel)
                addView(adminStatus)
                addView(admin)
                addView(networkStatus)
                addView(network)
                addView(networkHint)
                addView(captureStatus)
                addView(capture)
                addView(captureHint)
                addView(removalHint)
                addView(hint)
            },
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )

        requestNotificationPermission()
    }

    override fun onResume() {
        super.onResume()
        // Re-read on resume: the user returns here straight from the settings
        // toggle, so this is the moment the state changes.
        refresh()
    }

    /**
     * Asks for notification permission, once, on Android 13+.
     *
     * The status service runs either way, but with the permission denied its
     * notification is not posted — and for a service whose entire output is that
     * notification, that is indistinguishable from it not running. Not gated on
     * anything: it is asked at the same point the user is being walked through
     * two far larger grants, which is the moment it costs least.
     *
     * The result is deliberately not handled. There is nothing to do about a
     * refusal except keep recording to the tamper log, which is where the
     * durable record lives anyway.
     * https://developer.android.com/develop/ui/views/notifications/notification-permission
     */
    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFICATIONS)
    }

    private fun refresh() {
        val state = protection.state()
        val serviceOn = isServiceEnabled()
        // Rounded up: "0 min left" on something that has not happened yet reads
        // as a stuck timer.
        val minutes = (state.remainingMillis + 59_999) / 60_000

        // The service answers first, and not as a formality: the mode can read
        // ARMED while the service has been turned off underneath it — by adb,
        // by safe mode, or by the user before arming — and saying "the screens
        // that remove this app are blocked" when nothing is running would be the
        // one failure worse than not guarding at all. Same rule as the
        // unsupported-device notice below.
        status.text = if (!serviceOn) {
            getString(R.string.status_service_off)
        } else when (state.phase) {
            ProtectionPhase.OFF -> getString(R.string.status_protection_off)
            ProtectionPhase.ARMED -> getString(R.string.status_protection_armed)
            ProtectionPhase.DISARM_PENDING -> getString(R.string.status_disarm_pending, minutes)
            ProtectionPhase.DISARM_READY -> getString(R.string.status_disarm_ready, minutes)
            ProtectionPhase.DISARMED ->
                resources.getQuantityString(R.plurals.status_disarmed, minutes.toInt(), minutes)
        }

        protectionAction.text = getString(
            when (state.phase) {
                ProtectionPhase.OFF, ProtectionPhase.DISARMED -> R.string.action_arm
                ProtectionPhase.ARMED -> R.string.action_request_disarm
                ProtectionPhase.DISARM_PENDING -> R.string.action_request_disarm
                ProtectionPhase.DISARM_READY -> R.string.action_confirm_disarm
            },
        )
        protectionAction.setOnClickListener {
            when (state.phase) {
                ProtectionPhase.OFF, ProtectionPhase.DISARMED -> protection.arm()
                ProtectionPhase.ARMED -> protection.requestDisarm()
                ProtectionPhase.DISARM_READY -> protection.confirmDisarm()
                // The wait is the mechanism; there is nothing to tap through.
                ProtectionPhase.DISARM_PENDING -> Unit
            }
            refresh()
        }
        // Deliberately inert rather than hidden during the cooldown: the user
        // can see exactly what they asked for and what it is waiting on.
        protectionAction.isEnabled = state.phase != ProtectionPhase.DISARM_PENDING
        // Arming is pointless until the service that does the guarding is on.
        protectionAction.visibility = if (serviceOn) View.VISIBLE else View.GONE

        cancel.visibility = when (state.phase) {
            ProtectionPhase.DISARM_PENDING, ProtectionPhase.DISARM_READY -> View.VISIBLE
            else -> View.GONE
        }

        removalHint.visibility = if (state.guardActive) View.VISIBLE else View.GONE

        // The route to the toggle that turns the whole thing off. Offered only
        // when there is nothing to protect or protection is already off — beside
        // an armed guard it would be a one-tap bypass, which is exactly what an
        // earlier version of this screen shipped.
        open.visibility = if (!serviceOn || !state.guardActive) View.VISIBLE else View.GONE

        // Say plainly when the settings screens are not guarded here. The guard
        // silently does nothing on an unrecognised build, and a user who assumes
        // otherwise is worse off than one who knows.
        val supported = SettingsProfiles.forManufacturer(Build.MANUFACTURER) != null
        coverage.text = if (supported) "" else getString(R.string.status_device_unsupported)

        // Every path that changes the mode comes back through here, so this is
        // also where the status notification is brought up to date. It matters:
        // the mode lives in SharedPreferences with nothing to announce a change,
        // so without this the notification would keep claiming protection was off
        // for up to a poll interval after the user armed it — on the one surface
        // whose job is to say what is true right now. Starting from a resumed
        // activity is also the one call site the foreground-start rules never
        // refuse, and it is a no-op while there is nothing to report.
        GuardStatusService.start(this)

        val adminOn = HolyBlockerAdminReceiver.isActive(this)
        adminStatus.text =
            getString(if (adminOn) R.string.admin_status_on else R.string.admin_status_off)
        // Nothing to offer once it is on — the way back out is the system screen.
        admin.visibility = if (adminOn) View.GONE else View.VISIBLE

        refreshNetworkGuard(state)
        refreshScreenCapture(state)
    }

    /**
     * The consent result for screen capture, which — unlike the VPN's — has to
     * be read.
     *
     * `VpnService.prepare` can be asked again afterwards and is the authority on
     * its own grant, so that flow ignores its result. A `MediaProjection` grant
     * has no such query: the result `Intent` **is** the grant, it is single-use
     * from Android 14, and dropping it here means the user consented to nothing.
     * https://developer.android.com/reference/android/media/projection/MediaProjectionManager#getMediaProjection(int,%20android.content.Intent)
     */
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_CAPTURE_CONSENT) return
        // A refusal is a legitimate answer and gets no second prompt: the button
        // stays on screen and the user can decide again when they mean to.
        if (resultCode != RESULT_OK || data == null) return
        ScreenCaptureService.start(this, resultCode, data)
    }

    /**
     * Brings screen capture in line with the mode.
     *
     * Asymmetric with the network guard on purpose, and the asymmetry is
     * Android's rather than ours: this can *stop* capture but can never start it
     * on its own, because the grant it would need does not outlive the session it
     * was given for. So there is nothing to restore after a reboot, and the
     * button reappears whenever [ScreenCaptureService.isRunning] says the session
     * is over — including after a stop from the system's cast chip.
     */
    private fun refreshScreenCapture(state: ProtectionState) {
        val running = ScreenCaptureService.isRunning
        // Not shouldRun(): with capture already running, this is the condition
        // under which it must be torn down, and the consent half of shouldRun is
        // implied by it running at all.
        if (running && !ScreenCapture.shouldRun(state, consentGranted = true)) {
            ScreenCaptureService.stop(this)
        }

        captureStatus.text = getString(
            if (running) R.string.capture_status_on else R.string.capture_status_off,
        )
        capture.visibility = if (running) View.GONE else View.VISIBLE
        captureHint.visibility = capture.visibility
    }

    /**
     * Brings the network guard in line with the mode and the VPN grant.
     *
     * Driven from here rather than from the store, because the grant is not ours
     * to remember: `VpnService.prepare` is the only authority on it, it can be
     * revoked from Settings or taken by another VPN app without this app being
     * told, and an activity resuming is the moment that is cheapest to re-read.
     * Everything about *whether* it should be up is [NetworkGuard.shouldRun].
     */
    private fun refreshNetworkGuard(state: ProtectionState) {
        val consented = NetworkGuardService.hasConsent(this)
        val shouldRun = NetworkGuard.shouldRun(state, consentGranted = consented)

        networkStatus.text = getString(
            if (shouldRun) R.string.network_status_on else R.string.network_status_off,
        )
        // Offered only when it is not already granted, and only once there is a
        // mode to enforce — the consent dialog is a heavy prompt to raise before
        // the user has armed anything.
        network.visibility = if (consented) View.GONE else View.VISIBLE
        networkHint.visibility = network.visibility

        if (shouldRun) NetworkGuardService.start(this) else NetworkGuardService.stop(this)
    }

    /**
     * The system activation prompt.
     *
     * `EXTRA_DEVICE_ADMIN` is required: without a real receiver named here the
     * screen cannot be opened at all, which is why the `DeviceAdminAdd` entry in
     * [SettingsProfiles] could not be verified before this receiver existed.
     * https://developer.android.com/reference/android/app/admin/DevicePolicyManager#ACTION_ADD_DEVICE_ADMIN
     */
    private fun addAdminIntent(): Intent =
        Intent(DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN).apply {
            putExtra(
                DevicePolicyManager.EXTRA_DEVICE_ADMIN,
                HolyBlockerAdminReceiver.componentName(this@MainActivity),
            )
            putExtra(
                DevicePolicyManager.EXTRA_ADD_EXPLANATION,
                getString(R.string.admin_add_explanation),
            )
        }

    private companion object {
        const val REQUEST_NOTIFICATIONS = 1
        const val REQUEST_VPN_CONSENT = 2
        const val REQUEST_CAPTURE_CONSENT = 3
    }

    private fun isServiceEnabled(): Boolean = AccessibilityServiceStatus.isEnabled(
        enabledServicesSetting = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
        ),
        packageName = packageName,
        serviceClassName = ScreenGuardService::class.java.name,
    )
}
