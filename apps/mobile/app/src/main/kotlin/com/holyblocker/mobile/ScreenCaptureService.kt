package com.holyblocker.mobile

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.SystemClock
import android.util.DisplayMetrics
import android.util.Log
import android.view.Display
import com.holyblocker.mobile.policy.CaptureSize
import com.holyblocker.mobile.policy.FrameGate
import com.holyblocker.mobile.policy.FrameOutcome
import com.holyblocker.mobile.policy.ScreenCapture
import com.holyblocker.mobile.policy.TamperEvent

/**
 * The capture half of the guard: a `MediaProjection` that turns the screen into
 * a stream of small frames, gated down to the few worth analysing.
 *
 * **What it does not yet do is classify them.** `packages/image-sandbox` — the
 * perceptual hash set, the OCR pass and the ONNX model — does not exist, so
 * frames go to [CountingFrameSink] and are dropped. This service is built now
 * because it is the half that cannot be got right without a device: the start
 * order, the consent lifetime and the callback contract below are all things the
 * emulator teaches and the docs only hint at.
 *
 * ### The start order is strict and is the most common way to get this wrong
 *
 * 1. `MediaProjectionManager.createScreenCaptureIntent()`, launched from an
 *    Activity — [MainActivity] owns this.
 * 2. The activity result: `resultCode` and `data`.
 * 3. **Only then** this service is started and reaches `startForeground` with
 *    the `mediaProjection` type.
 * 4. **Only then** `getMediaProjection(resultCode, data)`.
 *
 * Inverting 3 and 4 throws on Android 10+, and it is worth being precise about
 * why the foreground service is not simply started first: an FGS of type
 * `mediaProjection` may only be started once consent exists, so the service is
 * both after the consent and before the projection, which is a narrower window
 * than it first looks.
 *
 * ### Consent is per session and cannot be cached
 *
 * There is no persistent grant to read back — nothing here can answer "is
 * capture permitted?" the way `VpnService.prepare` can. The result token is
 * single-use from Android 14: a second `getMediaProjection` with the same
 * `data` throws, so a restart means a fresh dialog. That is why [isRunning] is a
 * process-lifetime flag rather than a stored fact, and why nothing restarts this
 * after a reboot the way `BootReceiver` restores the VPN.
 *
 * It also means the user can end capture at any time from the system's own cast
 * chip. Like every other removal path in this product, that is recorded
 * ([TamperEvent.SCREEN_CAPTURE_REVOKED]) and not resisted — see `plan.md` §7.
 *
 * ### Reference documents
 *
 * - [`MediaProjection`](https://developer.android.com/reference/android/media/projection/MediaProjection)
 *   — in particular `registerCallback`, which **must** be called before
 *   `createVirtualDisplay` on Android 14+ or the projection throws.
 * - [`MediaProjectionManager`](https://developer.android.com/reference/android/media/projection/MediaProjectionManager)
 * - [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types)
 *   — the `mediaProjection` type and its start-order requirement.
 * - [`ImageReader`](https://developer.android.com/reference/android/media/ImageReader)
 *   — `acquireLatestImage`, and why an unclosed image stalls the stream.
 */
class ScreenCaptureService : Service() {

    private lateinit var protection: ProtectionStore
    private lateinit var tamperLog: TamperLogStore

    private val gate = FrameGate()
    private val sink: FrameSink = CountingFrameSink()

    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var reader: ImageReader? = null

    /**
     * Frames arrive off the main thread.
     *
     * The reduction in [onFrame] is arithmetic over a quarter-megabyte buffer,
     * and the analysis behind it will be a model pass. Neither belongs on the
     * thread that draws the UI of every other app on the device.
     */
    private var frameThread: HandlerThread? = null

    /** Reused across frames: the callback thread is the only writer. */
    private var planeBuffer: ByteArray = ByteArray(0)

    /**
     * The user stopped the projection, or the system did.
     *
     * Registered before the virtual display exists — required on Android 14+,
     * and correct on every version, since a projection can be stopped before the
     * display is up.
     */
    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() {
            Log.i(TAG, "the projection was stopped")
            teardown(TamperEvent.SCREEN_CAPTURE_REVOKED)
            stopSelf()
        }
    }

    override fun onCreate() {
        super.onCreate()
        protection = ProtectionStore(this)
        tamperLog = TamperLogStore.of(this)
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            teardown(TamperEvent.SCREEN_CAPTURE_STOPPED)
            stopSelf()
            return START_NOT_STICKY
        }

        // First, and before anything that can throw or return early: a service
        // started with startForegroundService and not reaching startForeground
        // within five seconds is killed with an ANR. It is also step 3 of the
        // start order — getMediaProjection below is illegal without it.
        enterForeground()

        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Activity.RESULT_CANCELED)
            ?: Activity.RESULT_CANCELED
        val resultData: Intent? = intent?.let(::resultData)

        if (resultCode != Activity.RESULT_OK || resultData == null) {
            // No consent token: nothing to project with, and none can be asked
            // for from here — only an Activity can raise the dialog.
            Log.w(TAG, "started without a consent result; stopping")
            isRunning = false
            stopSelf()
            return START_NOT_STICKY
        }

        if (!ScreenCapture.shouldRun(protection.state(), consentGranted = true)) {
            isRunning = false
            stopSelf()
            return START_NOT_STICKY
        }

        if (projection != null) return START_NOT_STICKY // already capturing

        return if (begin(resultCode, resultData)) {
            // START_NOT_STICKY, unlike the status service: a redelivered start
            // after a process kill would carry a spent consent token, and
            // getMediaProjection would throw on it. A killed capture session is
            // simply over, and the user grants a new one.
            START_NOT_STICKY
        } else {
            isRunning = false
            stopSelf()
            START_NOT_STICKY
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        teardown(TamperEvent.SCREEN_CAPTURE_STOPPED)
        super.onDestroy()
    }

    private fun begin(resultCode: Int, resultData: Intent): Boolean {
        val manager = getSystemService(MediaProjectionManager::class.java) ?: return false

        val media = try {
            manager.getMediaProjection(resultCode, resultData)
        } catch (e: Exception) {
            // IllegalStateException for a spent token, SecurityException when the
            // foreground service is not up or has the wrong type. Both are
            // programmer errors in the start order above, and neither may take
            // the process down.
            Log.e(TAG, "could not obtain the projection", e)
            null
        } ?: return false

        projection = media

        val thread = HandlerThread("screen-capture").apply { start() }
        frameThread = thread
        val handler = Handler(thread.looper)

        // Before createVirtualDisplay. Android 14 raises an IllegalStateException
        // otherwise, and the callback is how a user-stopped projection is noticed
        // on every version.
        media.registerCallback(projectionCallback, handler)

        val metrics = displayMetrics()
        val size = ScreenCapture.captureSize(metrics.widthPixels, metrics.heightPixels)

        val imageReader = ImageReader.newInstance(
            size.width,
            size.height,
            PixelFormat.RGBA_8888,
            IMAGE_BUFFER_COUNT,
        )
        imageReader.setOnImageAvailableListener({ onFrame(it, size) }, handler)
        reader = imageReader

        virtualDisplay = try {
            media.createVirtualDisplay(
                VIRTUAL_DISPLAY_NAME,
                size.width,
                size.height,
                metrics.densityDpi,
                // AUTO_MIRROR is what makes this the screen rather than an empty
                // display of our own; PUBLIC lets it mirror content from other
                // apps, which is the entire point.
                // https://developer.android.com/reference/android/hardware/display/DisplayManager#VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR or
                    DisplayManager.VIRTUAL_DISPLAY_FLAG_PUBLIC,
                imageReader.surface,
                null,
                handler,
            )
        } catch (e: Exception) {
            Log.e(TAG, "could not create the virtual display", e)
            null
        }

        if (virtualDisplay == null) return false

        // The dimensions, never the content: a capture session starting is the
        // kind of thing a user should be able to account for afterwards, and the
        // size is what explains a later "why is this frame letterboxed".
        Log.i(TAG, "capturing at ${size.width}x${size.height}")
        isRunning = true
        tamperLog.record(TamperEvent.SCREEN_CAPTURE_STARTED)
        return true
    }

    /**
     * One frame: reduce, gate, and hand on only what survives.
     *
     * `acquireLatestImage` rather than `acquireNextImage` on purpose — under load
     * the queue is drained to the newest frame and the older ones are discarded,
     * which is what the gate would do with them anyway and avoids analysing a
     * screen the user has already left.
     */
    private fun onFrame(imageReader: ImageReader, size: CaptureSize) {
        val image = try {
            imageReader.acquireLatestImage()
        } catch (e: Exception) {
            // The reader was closed between the callback and this call.
            null
        } ?: return

        try {
            val plane = image.planes[0]
            val buffer = plane.buffer
            val length = buffer.remaining()
            if (planeBuffer.size < length) planeBuffer = ByteArray(length)
            buffer.get(planeBuffer, 0, length)

            val grid = ScreenCapture.lumaGrid(
                plane = planeBuffer,
                width = image.width,
                height = image.height,
                rowStride = plane.rowStride,
                pixelStride = plane.pixelStride,
            )
            val hash = ScreenCapture.dHash(grid)
            val now = SystemClock.elapsedRealtime()

            // elapsedRealtime, not wall time: the gate's interval must not be
            // moved by the user changing the clock.
            if (gate.onFrame(hash, now) !is FrameOutcome.Analyse) return

            sink.accept(
                CapturedFrame(
                    width = image.width,
                    height = image.height,
                    pixels = ScreenCapture.packRgba(
                        plane = planeBuffer,
                        width = image.width,
                        height = image.height,
                        rowStride = plane.rowStride,
                        pixelStride = plane.pixelStride,
                    ),
                    hash = hash,
                    capturedAtMillis = now,
                ),
            )
        } catch (e: Exception) {
            // A frame is not worth the process. The next one is along in 16ms.
            Log.d(TAG, "dropped a frame: ${e.javaClass.simpleName}")
        } finally {
            // Not optional: an unclosed image holds a buffer, and the reader
            // stops delivering once all of them are held.
            image.close()
        }
    }

    /**
     * Stops the capture and records why, at most once.
     *
     * Reached from an explicit stop, from the projection callback, and from
     * `onDestroy` after either, so it has to be idempotent — the `projection ==
     * null` check is what makes it so.
     */
    private fun teardown(reason: TamperEvent) {
        val media = projection ?: return
        projection = null
        isRunning = false

        virtualDisplay?.release()
        virtualDisplay = null

        reader?.setOnImageAvailableListener(null, null)
        reader?.close()
        reader = null

        // Unregistered before stop(): stop() invokes the callback, and a
        // teardown re-entered through onStop would be a second log entry for one
        // event — which the idempotence check above would swallow, but only by
        // luck of ordering.
        media.unregisterCallback(projectionCallback)
        media.stop()

        frameThread?.quitSafely()
        frameThread = null
        planeBuffer = ByteArray(0)

        gate.reset()
        // A count, not a history. It is the one number that says whether the
        // gate did its job — a session measured in minutes that analysed
        // hundreds of frames means the throttle is not working — and it says
        // nothing whatever about what was on the screen.
        Log.i(TAG, "capture ended: ${(sink as? CountingFrameSink)?.accepted ?: 0} frames analysed")
        tamperLog.record(reason)
    }

    private fun enterForeground() {
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(): Notification {
        val text = getString(R.string.notification_screen_scan)
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setContentIntent(openApp())
            .setOngoing(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_STATUS)
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
            getString(R.string.channel_capture_name),
            // LOW: permanent while capture runs, and a status that interrupts is
            // one the user learns to silence. Same reasoning as the status
            // channel; a separate channel so it can be silenced on its own.
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.channel_capture_description)
            setShowBadge(false)
        }
        notificationManager()?.createNotificationChannel(channel)
    }

    private fun notificationManager(): NotificationManager? =
        getSystemService(NotificationManager::class.java)

    /**
     * Size of the real display, in pixels.
     *
     * `getRealMetrics` is deprecated at API 31 in favour of
     * `WindowManager.getCurrentWindowMetrics`, which needs a **visual** context
     * — an Activity — and this is a Service. `DisplayManager` is the supported
     * way to reach a display from a non-visual context, so the deprecation is
     * accepted deliberately rather than by omission.
     */
    @Suppress("DEPRECATION")
    private fun displayMetrics(): DisplayMetrics {
        val metrics = DisplayMetrics()
        val display = getSystemService(DisplayManager::class.java)
            ?.getDisplay(Display.DEFAULT_DISPLAY)
        display?.getRealMetrics(metrics)
        if (metrics.widthPixels == 0 || metrics.heightPixels == 0) {
            // No display to read: fall back to the configuration's own metrics
            // rather than handing a zero to ImageReader, which rejects it.
            return resources.displayMetrics
        }
        return metrics
    }

    @Suppress("DEPRECATION")
    private fun resultData(intent: Intent): Intent? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(EXTRA_RESULT_DATA, Intent::class.java)
        } else {
            intent.getParcelableExtra(EXTRA_RESULT_DATA)
        }

    companion object {
        private const val TAG = "ScreenCapture"
        private const val CHANNEL_ID = "screen_capture"

        /** Distinct from the status service's, which is 1. */
        private const val NOTIFICATION_ID = 2

        private const val VIRTUAL_DISPLAY_NAME = "holy-blocker-capture"

        private const val ACTION_STOP = "com.holyblocker.mobile.action.STOP_SCREEN_CAPTURE"
        private const val EXTRA_RESULT_CODE = "com.holyblocker.mobile.extra.RESULT_CODE"
        private const val EXTRA_RESULT_DATA = "com.holyblocker.mobile.extra.RESULT_DATA"

        /**
         * Images the reader keeps in flight.
         *
         * Two: one being read while the next is composed. More would only queue
         * frames the gate is about to discard, and every held image is a buffer
         * the display pipeline cannot reuse.
         */
        private const val IMAGE_BUFFER_COUNT = 2

        /**
         * Whether a capture session is running **in this process**.
         *
         * Not a grant and not persisted, because there is nothing to persist: a
         * `MediaProjection` consent lives for one session, so a fresh process
         * genuinely has no capture and no way to resume one. The UI reads this to
         * decide whether to offer the consent dialog again.
         *
         * **Set by [start], before the service has started**, which is not
         * fastidiousness: `onActivityResult` runs before `onResume`, so the
         * activity re-reads this flag in the same millisecond the start is
         * dispatched and long before `startForeground` has run. Setting it in
         * [begin] instead left the screen saying scanning was off with a live
         * projection behind it — observed on an android-36 emulator, and the exact
         * misreport the copy on that screen exists to avoid. Every path that
         * refuses or ends a session clears it.
         */
        @Volatile
        var isRunning: Boolean = false
            private set

        /** The system consent dialog. Must be launched from an Activity. */
        fun captureIntent(context: Context): Intent? =
            context.getSystemService(MediaProjectionManager::class.java)
                ?.createScreenCaptureIntent()

        /**
         * Starts capture with a consent result that has just been granted.
         *
         * **Never allowed to throw**, on the same rule as the other two services:
         * every caller is doing something that matters more, and capture is the
         * newest and most easily refused part of the product.
         *
         * @param resultData the `Intent` from `onActivityResult`. Single-use from
         *   Android 14 — it cannot be stored and replayed later.
         */
        fun start(context: Context, resultCode: Int, resultData: Intent) {
            val intent = Intent(context, ScreenCaptureService::class.java)
                .putExtra(EXTRA_RESULT_CODE, resultCode)
                .putExtra(EXTRA_RESULT_DATA, resultData)
            isRunning = true
            try {
                context.startForegroundService(intent)
            } catch (e: Exception) {
                isRunning = false
                // ForegroundServiceStartNotAllowedException is API 31+ and is
                // caught by supertype. It should not happen here — this is called
                // straight out of an activity result — but a refused capture must
                // not take the guard with it.
                Log.w(TAG, "could not start screen capture", e)
            }
        }

        /** Stops capture. Safe to call when it is not running. */
        fun stop(context: Context) {
            if (!isRunning) return
            try {
                context.startService(
                    Intent(context, ScreenCaptureService::class.java).setAction(ACTION_STOP),
                )
            } catch (e: Exception) {
                Log.w(TAG, "could not stop screen capture", e)
            }
        }
    }
}
