package com.holyblocker.mobile

import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.os.Build
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import com.holyblocker.mobile.policy.CoverState

/**
 * The cover mechanism: a `SYSTEM_ALERT_WINDOW` view drawn over whatever app is
 * in front.
 *
 * Per docs/decisions/content-interception.md this is Layer 2's only way to
 * obscure content on Android — there is no compositor hook available to a
 * non-root app.
 *
 * All methods must be called on the main thread; `WindowManager` requires it.
 */
class OverlayController(private val context: Context) {

    private val windowManager =
        context.getSystemService(Context.WINDOW_SERVICE) as WindowManager

    private var view: View? = null
    private var shownState: CoverState = CoverState.CLEAR

    /**
     * `TYPE_ACCESSIBILITY_OVERLAY` is the only overlay type an accessibility
     * service can show without the user granting "display over other apps"
     * separately, and it sits above `TYPE_APPLICATION_OVERLAY`. It exists from
     * API 22; below that we fall back to the permission-gated type.
     */
    private val overlayType: Int
        get() = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP_MR1) {
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY
        } else {
            @Suppress("DEPRECATION")
            WindowManager.LayoutParams.TYPE_SYSTEM_ERROR
        }

    fun apply(state: CoverState) {
        if (state == shownState) return
        val applied = when (state) {
            CoverState.COVER -> show(opaque = true)
            CoverState.WARN -> show(opaque = false)
            CoverState.CLEAR -> {
                hide()
                true
            }
        }

        // Only record a state that actually took. A failed `addView` leaving
        // `shownState` at COVER would make `apply` believe a cover is up that is
        // not, and the early return above would then suppress every retry — the
        // screen stays uncovered for as long as the user keeps it open. Leaving
        // it CLEAR costs one redundant attempt per event and self-heals.
        if (applied) shownState = state
    }

    private fun show(opaque: Boolean): Boolean {
        hide()

        val label = TextView(context).apply {
            // The formation model forbids text that renders a judgement about
            // the person; this names only what the tool did.
            text = context.getString(
                if (opaque) R.string.overlay_covered else R.string.overlay_warn,
            )
            setTextColor(Color.WHITE)
            textSize = 18f
            gravity = Gravity.CENTER
        }

        val container = FrameLayout(context).apply {
            setBackgroundColor(if (opaque) COVER_COLOR else WARN_COLOR)
            addView(
                label,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    Gravity.CENTER,
                ),
            )
            // An opaque cover must swallow touches so the content underneath
            // cannot be interacted with blind; the warn tint stays passive.
            isClickable = opaque
            isFocusable = false
        }

        var flags = WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
        if (!opaque) {
            flags = flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        }

        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.MATCH_PARENT,
            overlayType,
            flags,
            PixelFormat.TRANSLUCENT,
        )

        // Every call here is on the accessibility callback path, and an uncaught
        // throw there kills the event — repeated, it takes the service down,
        // which is a bypass by way of a crash. WindowManager throws for reasons
        // outside this class's control: the token is rejected once the service
        // is being unbound, and the window can already be gone.
        return try {
            windowManager.addView(container, params)
            view = container
            true
        } catch (e: WindowManager.BadTokenException) {
            Log.w(TAG, "overlay rejected by the window manager", e)
            view = null
            false
        } catch (e: IllegalStateException) {
            // "View has already been added to the window manager" — the view is
            // fresh here, so this means our own bookkeeping drifted.
            Log.w(TAG, "overlay could not be attached", e)
            view = null
            false
        }
    }

    private fun hide() {
        val attached = view
        view = null
        if (attached == null) return
        try {
            windowManager.removeView(attached)
        } catch (e: IllegalArgumentException) {
            // "View not attached to window manager". The window is gone, which
            // is the state this was asking for, so there is nothing to recover —
            // but it must not propagate: see the note in `show`.
            Log.w(TAG, "overlay was already detached", e)
        }
    }

    /** Releases the window; call from the service's teardown. */
    fun destroy() {
        hide()
        shownState = CoverState.CLEAR
    }

    companion object {
        private const val TAG = "ScreenGuard"

        private val COVER_COLOR = Color.rgb(18, 20, 24)
        private val WARN_COLOR = Color.argb(140, 18, 20, 24)

        /**
         * Only needed for the `TYPE_APPLICATION_OVERLAY` fallback path — an
         * accessibility overlay does not require this grant. Surfaced for the
         * onboarding screen.
         */
        fun canDrawOverlays(context: Context): Boolean =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                Settings.canDrawOverlays(context)
            } else {
                true
            }
    }
}
