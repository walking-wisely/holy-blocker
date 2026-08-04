#!/usr/bin/env bash
# End-to-end smoke test for the Android capture path (ScreenCaptureService).
#
# Proves the seam unit tests cannot reach: a real MediaProjection, a real
# VirtualDisplay, and real ImageReader frames arriving with the strides and
# dimensions the platform chose rather than the ones a test made up.
#
# What it asserts:
#   1. The start order works — consent, then foreground service, then
#      getMediaProjection — and the service really is a mediaProjection-typed FGS
#      with its own notification posted and the start recorded.
#   2. Frames arrive and reach the sink, at the size ScreenCapture.captureSize
#      derives from the display.
#   3. **The app says scanning is on while it is on.** A regression test: the
#      first version set the running flag in the service, which resumes after
#      onActivityResult, so the screen reported "off" over a live projection.
#   4. Stopping the projection from the system's own chip is noticed, recorded as
#      revoked, and tears the session down.
#   5. The gate throttles. The frame count for the whole session must be a
#      handful, not thousands — this is what makes the capture path safe to leave
#      running on a phone, and it is the assertion that would catch a rate cap
#      wired up backwards.
#   6. A reboot does not bring capture back, and the app does not claim it did. A
#      MediaProjection token is single-use and per session, so unlike the VPN
#      there is nothing for BootReceiver to restore.
#
# Two things this cannot reach, both recorded in plan.md §6 rather than skipped
# quietly: the disarm-driven stop (flipping the mode needs the process down,
# which kills the projection first) and the appop being revoked mid-session
# (checked at grant time only — denying PROJECT_MEDIA does not stop a live
# projection, measured here).
#
# The consent dialog is a system window a script cannot tap, so the PROJECT_MEDIA
# appop stands in for accepting it: with the op allowed the permission activity
# returns RESULT_OK without showing UI. Same substitution smoke-test-vpn.sh makes
# with ACTIVATE_VPN.
#
# Usage: smoke-test-capture.sh   (needs a booted emulator or device on adb)
#
# It writes the protection mode directly through `run-as`, which needs a
# debuggable build — i.e. the debug APK, which is what this installs.
set -euo pipefail

pkg="com.holyblocker.mobile"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_dir="$(dirname "$here")"
apk="$mobile_dir/app/build/outputs/apk/debug/app-debug.apk"

# The ceiling for assertion 5. At the gate's one-per-second cap, a session spent
# on a static screen cannot honestly approach this.
max_frames_static=20

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

wait_for_boot() {
    adb wait-for-device
    until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
        sleep 2
    done
}

capture_running() {
    adb shell dumpsys activity services "$pkg" 2>/dev/null | grep -q "ScreenCaptureService"
}

dump_ui() {
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || fail "uiautomator dump failed"
    adb shell cat /sdcard/ui.xml
}

# Taps a control by its label, read out of a uiautomator dump.
#
# By label rather than by coordinates: the onboarding screen is a plain vertical
# LinearLayout with no ids, so inserting one control moves every one below it and
# a hardcoded tap would quietly start hitting the wrong thing.
tap_label() {
    local label="$1" nums
    nums="$(dump_ui | tr '<' '\n' | grep -F "text=\"$label\"" \
        | grep -o 'bounds="[^"]*"' | head -1 | grep -o '[0-9]\+' | paste -sd' ' -)"
    [[ -n "$nums" ]] || fail "no control labelled '$label' on screen"
    # bounds="[x1,y1][x2,y2]" -> the centre of the rectangle.
    # shellcheck disable=SC2086
    set -- $nums
    adb shell input tap $(( ($1 + $3) / 2 )) $(( ($2 + $4) / 2 ))
}

arm() {
    # Written with the process down: SharedPreferences caches in memory, so a
    # file edit under a running app is simply overwritten.
    adb shell am force-stop "$pkg"
    adb shell "run-as $pkg sh -c 'cat > shared_prefs/protection_mode.xml' <<'EOF'
<?xml version='1.0' encoding='utf-8' standalone='yes' ?>
<map>
    <boolean name=\"armed\" value=\"$1\" />
</map>
EOF"
}

[[ -f "$apk" ]] || fail "no APK at $apk — run ./gradlew :app:assembleDebug"

echo "==> waiting for device"
wait_for_boot

echo "==> installing"
adb install -r -g "$apk" >/dev/null || fail "install failed"

echo "==> granting the projection op"
adb shell appops set "$pkg" PROJECT_MEDIA allow

echo "==> arming protection"
arm true

echo "==> starting capture"
adb logcat -c
adb shell am start -n "$pkg/.MainActivity" >/dev/null 2>&1
sleep 3
tap_label "TURN ON SCREEN SCANNING"
sleep 5

capture_running || fail "ScreenCaptureService is not running"
services="$(adb shell dumpsys activity services "$pkg")"
grep -A6 "ScreenCaptureService" <<<"$services" | grep -q "types=0x00000020" \
    || fail "the service is not a mediaProjection-typed foreground service"
grep -A6 "ScreenCaptureService" <<<"$services" | grep -q "channel=screen_capture" \
    || fail "the capture notification was not posted"
adb shell "run-as $pkg tail -3 files/tamper-log.tsv" | grep -q "capture_on" \
    || fail "the capture start was not recorded in the tamper log"
echo "    projection up, foreground, and recorded"

echo "==> checking frames arrive"
adb logcat -d -s ScreenCapture:V | grep -qE "capturing at [0-9]+x[0-9]+" \
    || fail "no virtual display was created"
adb logcat -d -s ScreenCapture:V | grep -q "analysed 1 frames" \
    || fail "no frame reached the sink — the reader or the gate is not wired up"
echo "    $(adb logcat -d -s ScreenCapture:V | grep -o 'capturing at [0-9]*x[0-9]*' | head -1)"

echo "==> checking the app reports it"
dump_ui | grep -q "Screen scanning is on" \
    || fail "the app says scanning is off while a projection is live"
echo "    the screen says scanning is on"

echo "==> leaving it on a static screen for 30s (the gate must throttle)"
sleep 30

echo "==> stopping from the system chip"
# The projection chip sits beside the clock in the status bar. It belongs to
# SystemUI and does not appear in a uiautomator dump of the foreground app, so
# this is the one tap here that has to be positional — the dialog it opens is
# then driven by label like everything else. A miss fails loudly below rather
# than passing quietly.
width="$(adb shell wm size | grep -o '[0-9]*x[0-9]*' | tail -1 | cut -dx -f1)"
for fraction in 27 22 32; do
    adb shell input tap $(( width * fraction / 100 )) 66
    sleep 2
    if dump_ui | grep -q "Stop sharing screen?"; then break; fi
done
dump_ui | grep -q "Stop sharing screen?" || fail "could not reach the system's stop-sharing dialog"
tap_label "Stop sharing"
sleep 4

! capture_running || fail "capture survived the system stop"
adb logcat -d -s ScreenCapture:V | grep -q "the projection was stopped" \
    || fail "MediaProjection.Callback.onStop did not fire"
adb shell "run-as $pkg tail -3 files/tamper-log.tsv" | grep -q "capture_revoked" \
    || fail "the stop was not recorded as a revoke"
echo "    stopped, noticed, and recorded"

echo "==> checking the gate throttled"
frames="$(adb logcat -d -s ScreenCapture:V | grep -o 'capture ended: [0-9]* frames' \
    | tail -1 | grep -o '[0-9]*')"
[[ -n "$frames" ]] || fail "the session did not report a frame count"
(( frames > 0 )) || fail "the session analysed nothing at all"
(( frames <= max_frames_static )) \
    || fail "$frames frames over a static session — the gate is not throttling"
echo "    $frames frames analysed over the whole session"

echo "==> rebooting (capture must not come back, and must not claim to)"
adb reboot
wait_for_boot
sleep 20
! capture_running || fail "capture came back after a reboot — a consent token cannot survive one"
adb shell am start -n "$pkg/.MainActivity" >/dev/null 2>&1
sleep 3
dump_ui | grep -q "Screen scanning is off" \
    || fail "the app claims scanning is on after a reboot"
echo "    gone after the reboot, and the app says so"

echo
echo "SMOKE PASS"
