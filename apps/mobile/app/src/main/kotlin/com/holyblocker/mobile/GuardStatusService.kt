package com.holyblocker.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.Settings
import android.util.Log
import com.holyblocker.mobile.policy.AccessibilityServiceStatus
import com.holyblocker.mobile.policy.GuardConditions
import com.holyblocker.mobile.policy.GuardHealth
import com.holyblocker.mobile.policy.GuardStatus
import com.holyblocker.mobile.policy.SettingsProfiles
import com.holyblocker.mobile.policy.StatusMessage

/**
 * The foreground service: an always-visible status surface, and the half of
 * tamper detection the guard cannot perform on itself.
 *
 * **It is not what keeps the guard running, and nothing here should be written as
 * if it were.** An `AccessibilityService` is bound by the system and is rebound
 * after a reboot for as long as it stays enabled, so this makes the guard neither
 * harder to kill nor able to survive anything it could not survive already. What
 * it adds is a process that is still alive *after* the guard stops:
 *
 * - **It notices a disable the guard cannot report.** `adb`, a guest session, or
 *   a settings screen an OEM build hides from the guard all end with the service
 *   gone and nothing written — see `backlog.md`, "Cannot be closed at Device
 *   Admin level". This service polls the same secure setting the onboarding
 *   screen parses and records the gap while it is open, rather than at whatever
 *   later point the guard happens to be enabled again.
 * - **It is the FGS host the last two plan steps need.** `VpnService` and
 *   `MediaProjection` both require one, and the `MediaProjection` start order is
 *   strict (plan.md §6).
 *
 * Every decision is in [GuardStatus] and unit tested on the JVM; this class holds
 * the notification, the clock and the observer.
 */
class GuardStatusService : Service() {

    private val handler = Handler(Looper.getMainLooper())
    private lateinit var protection: ProtectionStore
    private lateinit var tamperLog: TamperLogStore

    /**
     * The last observed health, so a fall is recorded on its edge rather than on
     * every poll. Null until the first check — which [GuardStatus.noticeOn]
     * treats as a fall, not as health.
     */
    private var lastHealth: GuardHealth? = null

    private val recheck = Runnable { check() }

    /**
     * Fires the moment the accessibility service is enabled or disabled, so the
     * poll below is a backstop rather than the mechanism.
     *
     * Worth the extra moving part: the poll interval is otherwise the delay
     * between the guard being switched off and the record of it, and a minute of
     * that is a minute in which nothing says the phone is unguarded.
     */
    private val settingsObserver = object : ContentObserver(handler) {
        override fun onChange(selfChange: Boolean) = check()
    }

    override fun onCreate() {
        super.onCreate()
        protection = ProtectionStore(this)
        tamperLog = TamperLogStore.of(this)
        createChannel()

        contentResolver.registerContentObserver(
            Settings.Secure.getUriFor(Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            false,
            settingsObserver,
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // First, and before anything that can throw: a service started with
        // startForegroundService that does not reach startForeground within five
        // seconds is killed with an ANR.
        // https://developer.android.com/develop/background-work/services/fgs
        enterForeground(conditions())

        check()

        // START_STICKY, not START_NOT_STICKY: a status surface that disappears
        // when the process is recycled says nothing about the guard, which is the
        // one thing it is here to report. Note it does *not* restart after a
        // force-stop — nothing does — and that case is covered by the tamper log
        // reading the session back as an unclean stop.
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        handler.removeCallbacks(recheck)
        contentResolver.unregisterContentObserver(settingsObserver)
        super.onDestroy()
    }

    /**
     * Reads the conditions once, records what changed, and re-posts itself.
     *
     * Called from the timer, from the settings observer, and on every start, so
     * it has to be idempotent — [GuardStatus.noticeOn] is what makes it so.
     */
    private fun check() {
        val conditions = conditions()

        val health = GuardStatus.health(conditions)
        GuardStatus.noticeOn(lastHealth, health)?.let { event ->
            Log.w(TAG, "protection is armed but the screen guard is not running")
            tamperLog.record(event)
        }
        lastHealth = health

        if (!GuardStatus.shouldKeepRunning(conditions)) {
            // Nothing armed and nothing scanning. stopSelf() takes the
            // notification with it.
            stopSelf()
            return
        }

        notificationManager()?.notify(NOTIFICATION_ID, buildNotification(conditions))

        handler.removeCallbacks(recheck)
        handler.postDelayed(recheck, GuardStatus.CHECK_INTERVAL_MILLIS)
    }

    private fun conditions() = GuardConditions(
        serviceEnabled = AccessibilityServiceStatus.isEnabled(
            enabledServicesSetting = Settings.Secure.getString(
                contentResolver,
                Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
            ),
            packageName = packageName,
            serviceClassName = ScreenGuardService::class.java.name,
        ),
        protection = protection.state(),
        deviceRecognised = SettingsProfiles.forManufacturer(Build.MANUFACTURER) != null,
    )

    private fun enterForeground(conditions: GuardConditions) {
        val notification = buildNotification(conditions)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            // Android 14+ requires a type, and the manifest declaration alone is
            // not enough when the call site can name one.
            // https://developer.android.com/develop/background-work/services/fgs/service-types
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(conditions: GuardConditions): Notification {
        // Rounded up: "0 min left" on something that has not happened yet reads
        // as a stuck timer. Same rule as MainActivity.
        val minutes = (conditions.protection.remainingMillis + 59_999) / 60_000

        val text = when (GuardStatus.message(conditions)) {
            StatusMessage.GUARD_NOT_RUNNING -> getString(R.string.notification_guard_not_running)
            StatusMessage.DISARM_PENDING -> getString(R.string.status_disarm_pending, minutes)
            StatusMessage.DISARM_READY -> getString(R.string.status_disarm_ready, minutes)
            StatusMessage.DISARMED ->
                resources.getQuantityString(R.plurals.status_disarmed, minutes.toInt(), minutes)
            StatusMessage.DEVICE_UNVERIFIED -> getString(R.string.status_device_unsupported)
            StatusMessage.PROTECTING -> getString(R.string.status_protection_armed)
            StatusMessage.IDLE -> getString(R.string.notification_protection_off)
        }

        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            // The copy runs past one line in several states, and a status the
            // user has to guess at is not a status.
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setContentIntent(openApp())
            // Ongoing, because it is: the service behind it is running. It also
            // keeps the one notification that says whether the phone is guarded
            // from being swiped away by reflex.
            .setOngoing(true)
            // A timestamp on a permanent notification only ever reads as stale.
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_STATUS)
            // Visible on the lock screen without its text: it says nothing about
            // what was on the screen, but there is no reason to broadcast the
            // state of someone's accountability tool to a room.
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .build()
    }

    private fun openApp(): PendingIntent {
        val intent = Intent(this, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(
            this,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.channel_status_name),
            // LOW: silent and no heads-up. This is on screen permanently, and a
            // status that interrupts is a status the user learns to dismiss.
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.channel_status_description)
            setShowBadge(false)
        }
        notificationManager()?.createNotificationChannel(channel)
    }

    private fun notificationManager(): NotificationManager? =
        getSystemService(NotificationManager::class.java)

    companion object {
        private const val TAG = "GuardStatus"
        private const val CHANNEL_ID = "guard_status"
        private const val NOTIFICATION_ID = 1

        /**
         * Starts the service if it is not already running.
         *
         * **Never allowed to throw.** Every caller is doing something else that
         * matters more — the guard connecting, the activity resuming, the device
         * booting — and Android 12+ rejects a foreground start from the
         * background with an exception. This service is a status surface; losing
         * it must not take the guard down with it.
         * https://developer.android.com/develop/background-work/services/foreground-services#background-start-restrictions
         */
        fun start(context: Context) {
            // Not started at all when there is nothing to report, so an install
            // that never gets past onboarding does not acquire a permanent
            // notification. The service stops itself on the same condition.
            val protection = ProtectionStore(context)
            if (!GuardStatus.shouldKeepRunning(
                    GuardConditions(
                        serviceEnabled = isGuardEnabled(context),
                        protection = protection.state(),
                        // Irrelevant to shouldKeepRunning; read here rather than
                        // plumbed so the conditions are built one way only.
                        deviceRecognised = SettingsProfiles.forManufacturer(Build.MANUFACTURER) != null,
                    ),
                )
            ) {
                return
            }

            val intent = Intent(context, GuardStatusService::class.java)
            try {
                context.startForegroundService(intent)
            } catch (e: Exception) {
                // ForegroundServiceStartNotAllowedException is API 31+, so it is
                // caught by supertype rather than named; the reaction is the same
                // either way.
                Log.w(TAG, "could not start the status service", e)
            }
        }

        private fun isGuardEnabled(context: Context): Boolean =
            AccessibilityServiceStatus.isEnabled(
                enabledServicesSetting = Settings.Secure.getString(
                    context.contentResolver,
                    Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
                ),
                packageName = context.packageName,
                serviceClassName = ScreenGuardService::class.java.name,
            )
    }
}
