package com.holyblocker.mobile.admin

import android.app.admin.DeviceAdminReceiver
import android.app.admin.DevicePolicyManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.util.Log
import com.holyblocker.mobile.R

/**
 * The device admin component.
 *
 * This receiver exists almost entirely to *be active*. At plain Device Admin
 * there is no policy call available to this product — see §7 of
 * `docs/components/mobile/plan.md` — so nothing here enforces anything. What an
 * active admin buys is framework behaviour we cannot get any other way:
 *
 * - **Uninstall is refused** while the admin is active ("This app is an active
 *   device administrator and must be deactivated before uninstalling"). That is
 *   the package manager's own check, not a policy call, so the Android 9
 *   device-admin deprecation does not touch it.
 * - **[onDisableRequested] fires before deactivation takes effect** — the last
 *   reliable moment to observe that removal is under way.
 *
 * Deliberately declares no policies. Every `<uses-policies>` tag would grant
 * powers the product has no reason to hold, and none of them is needed for
 * either property above.
 *
 * https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver
 */
class HolyBlockerAdminReceiver : DeviceAdminReceiver() {

    override fun onEnabled(context: Context, intent: Intent) {
        Log.i(TAG, "device admin enabled — uninstall now requires deactivation first")
    }

    /**
     * Fires after the user confirms deactivation, before it takes effect.
     *
     * The returned string is shown on the confirmation screen. It states the
     * consequence and stops there: per `docs/decisions/formation-model.md` this
     * copy describes what happens, it does not appeal to conscience or attempt
     * to talk the person out of it. Obstructing here is not an option the API
     * offers and would not be taken if it were — the exit path staying open is
     * what makes this an accountability tool rather than a trap.
     */
    override fun onDisableRequested(context: Context, intent: Intent): CharSequence =
        context.getString(R.string.admin_disable_warning)

    override fun onDisabled(context: Context, intent: Intent) {
        // Terminal for this component: once disabled, uninstall is unblocked and
        // nothing here runs again. Worth a log line because it is the one moment
        // a still-running accessibility service can notice the change.
        Log.w(TAG, "device admin disabled — uninstall is no longer blocked")
    }

    companion object {
        private const val TAG = "HolyBlockerAdmin"

        fun componentName(context: Context): ComponentName =
            ComponentName(context.applicationContext, HolyBlockerAdminReceiver::class.java)

        /** Whether the admin is active — what `SettingsGuard` gates on. */
        fun isActive(context: Context): Boolean {
            val dpm = context.getSystemService(Context.DEVICE_POLICY_SERVICE) as? DevicePolicyManager
                ?: return false
            return dpm.isAdminActive(componentName(context))
        }
    }
}
