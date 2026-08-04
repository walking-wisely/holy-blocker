package com.holyblocker.mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Brings the status service and the network guard back after a restart or an app
 * update.
 *
 * **It writes nothing to the tamper log, and that is the whole design of this
 * class.** `ACTION_BOOT_COMPLETED` is not evidence of a boot: it is delivered to
 * this app every time it leaves the force-stopped state, reproduced twice on an
 * android-36 emulator three minutes into its uptime. An earlier version recorded
 * a `BOOT` entry here, and `TamperLog.classifyConnect` trusted it — so a
 * force-stop, the removal-shaped event that classification exists to catch, wrote
 * its own alibi and read back as an ordinary restart. Nothing the system can
 * deliver outside a boot may stand in for one; the monotonic clock going
 * backwards is the only evidence there is.
 *
 * **The accessibility guard does not need this to survive a reboot.** The system
 * rebinds an enabled accessibility service on its own. Two other things do not
 * come back by themselves and are restored here:
 *
 * - the status surface, because a stopped foreground service is not brought back
 *   by anything else;
 * - the network guard, because Android does **not** re-establish a `VpnService`
 *   after a restart — that is what `setAlwaysOnVpnPackage` is for, and it is
 *   device-owner only, which this product deliberately is not (`plan.md` §7).
 *   Measured on an android-36 emulator: with protection armed and the VPN up
 *   before a reboot, `tun0` was absent afterwards and a blocked name resolved
 *   normally. The user would have been unfiltered until they next opened the
 *   app, with nothing on screen saying so.
 *
 * Both calls are gated on their own conditions and neither may throw; a restore
 * that fails must not take the other one down with it.
 *
 * #### Reference documents
 *
 * - [`ACTION_BOOT_COMPLETED`](https://developer.android.com/reference/android/content/Intent#ACTION_BOOT_COMPLETED)
 * - [`ACTION_MY_PACKAGE_REPLACED`](https://developer.android.com/reference/android/content/Intent#ACTION_MY_PACKAGE_REPLACED)
 * - [Background start restrictions](https://developer.android.com/develop/background-work/services/foreground-services#background-start-restrictions)
 *   — both broadcasts above are exemptions, which is why a foreground start is
 *   permitted from here at all
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> {
                Log.i(TAG, "restoring services after ${intent.action}")
                GuardStatusService.start(context)
                // No-op unless protection is armed and VPN consent survives —
                // both are checked inside, and the consent is the user's to
                // revoke at any time.
                NetworkGuardService.start(context)
            }

            else -> Unit
        }
    }

    private companion object {
        const val TAG = "BootReceiver"
    }
}
