package com.holyblocker.mobile

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import com.holyblocker.mobile.policy.AccessibilityServiceStatus

/**
 * Onboarding surface. The MVP has one job: get the user to the accessibility
 * toggle and explain the Restricted Settings detour that a sideloaded app hits
 * on Android 13+ (see docs/decisions/content-interception.md — sideload friction
 * is intentional here, and the detour is authenticated by the device PIN).
 */
class MainActivity : Activity() {

    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        status = TextView(this).apply { textSize = 18f }

        val hint = TextView(this).apply {
            text = getString(R.string.restricted_settings_hint)
            textSize = 14f
            setTextColor(Color.GRAY)
        }

        val open = Button(this).apply {
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
                addView(open)
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
        status.text = getString(
            if (isServiceEnabled()) R.string.status_service_on else R.string.status_service_off,
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
