package com.holyblocker.mobile

import android.app.Activity
import android.app.admin.DevicePolicyManager
import android.content.Intent
import android.graphics.Color
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
import com.holyblocker.mobile.policy.ProtectionPhase
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
                addView(removalHint)
                addView(hint)
            },
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
    }

    override fun onResume() {
        super.onResume()
        // Re-read on resume: the user returns here straight from the settings
        // toggle, so this is the moment the state changes.
        refresh()
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

        val adminOn = HolyBlockerAdminReceiver.isActive(this)
        adminStatus.text =
            getString(if (adminOn) R.string.admin_status_on else R.string.admin_status_off)
        // Nothing to offer once it is on — the way back out is the system screen.
        admin.visibility = if (adminOn) View.GONE else View.VISIBLE
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

    private fun isServiceEnabled(): Boolean = AccessibilityServiceStatus.isEnabled(
        enabledServicesSetting = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
        ),
        packageName = packageName,
        serviceClassName = ScreenGuardService::class.java.name,
    )
}
