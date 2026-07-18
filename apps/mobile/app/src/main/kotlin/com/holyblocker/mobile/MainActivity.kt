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
import com.holyblocker.mobile.policy.ReleasePhase
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
    private lateinit var release: Button
    private lateinit var adminStatus: TextView
    private lateinit var admin: Button
    private lateinit var suspension: GuardSuspension

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        suspension = GuardSuspension(this)
        status = TextView(this).apply { textSize = 18f }

        coverage = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        release = Button(this).apply {
            setOnClickListener {
                suspension.requestRelease()
                refresh()
            }
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
                addView(release)
                addView(adminStatus)
                addView(admin)
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
        val state = suspension.state()
        val serviceOn = isServiceEnabled()
        val minutes = (state.remainingMillis + 59_999) / 60_000
        val seconds = (state.remainingMillis + 999) / 1000

        status.text = when (state.phase) {
            ReleasePhase.PENDING -> getString(R.string.status_release_pending, minutes)
            ReleasePhase.OPEN -> getString(R.string.status_release_open, seconds)
            ReleasePhase.IDLE ->
                getString(if (serviceOn) R.string.status_service_on else R.string.status_service_off)
        }

        release.text = getString(
            when (state.phase) {
                ReleasePhase.IDLE -> R.string.action_request_release
                else -> R.string.action_cancel_release
            },
        )
        release.setOnClickListener {
            if (state.phase == ReleasePhase.IDLE) suspension.requestRelease() else suspension.clear()
            refresh()
        }
        // No point offering a release when nothing is being guarded yet.
        release.visibility = if (serviceOn) View.VISIBLE else View.GONE

        open.visibility =
            if (!serviceOn || state.phase == ReleasePhase.OPEN) View.VISIBLE else View.GONE

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
