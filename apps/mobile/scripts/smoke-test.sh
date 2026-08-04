#!/usr/bin/env bash
# End-to-end smoke test for the Android text path.
#
# Proves the seam that unit tests cannot reach: the generated UniFFI bindings
# calling the real libtext_policy_ffi.so on a device, driving a real
# AccessibilityService and overlay.
#
# What it asserts:
#   1. The service connects — i.e. the .so loaded and PolicyEngine constructed.
#   2. Benign text in another app produces ALLOW / CLEAR.
#   3. Text matching the dictionary produces BLOCK / COVER.
#   4. An overlay window is actually present in the window manager.
#
# Usage: smoke-test.sh [avd-name]   (needs a booted emulator or device on adb)
set -euo pipefail

avd="${1:-holyblocker-test}"
pkg="com.holyblocker.mobile"
svc="$pkg/$pkg.ScreenGuardService"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_dir="$(dirname "$here")"
apk="$mobile_dir/app/build/outputs/apk/debug/app-debug.apk"

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

[[ -f "$apk" ]] || fail "no APK at $apk — run ./gradlew :app:assembleDebug"

echo "==> waiting for device"
adb wait-for-device
# sys.boot_completed flips before the launcher is usable; polling it is the
# standard way to avoid installing into a half-booted system.
until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
    sleep 2
done

echo "==> installing"
adb install -r -g "$apk" >/dev/null || fail "install failed"

echo "==> enabling the accessibility service"
# Writing the setting directly is the emulator equivalent of the user toggling
# it. On a sideloaded install a real device would first require the Restricted
# Settings detour (App info -> ⋮ -> Allow restricted settings), which is
# deliberate — see docs/decisions/content-interception.md.
#
# Retried and verified: shortly after first boot the system rewrites the
# accessibility defaults, silently clobbering a write that reported success.
adb logcat -c
enabled=""
for _ in $(seq 1 10); do
    adb shell settings put secure enabled_accessibility_services "$svc" >/dev/null
    adb shell settings put secure accessibility_enabled 1 >/dev/null
    sleep 2
    if [[ "$(adb shell settings get secure enabled_accessibility_services | tr -d '\r')" == "$svc" ]]; then
        enabled=1
        break
    fi
done
[[ -n "$enabled" ]] || fail "could not enable $svc (setting kept reverting)"

echo "==> 1. service connected (proves the .so loaded)"
# Poll rather than sleep: binding takes a variable moment, and a fixed wait
# either flakes or wastes time.
connected=""
for _ in $(seq 1 15); do
    if adb logcat -d -s ScreenGuard | grep -q "screen guard connected"; then
        connected=1
        break
    fi
    sleep 2
done
if [[ -z "$connected" ]]; then
    adb logcat -d | grep -iE "UnsatisfiedLink|dlopen|FATAL" | tail -5 >&2 || true
    fail "service never connected — the native library likely failed to load"
fi
echo "    ok"

# `|| true`: an empty log is the normal state while polling, but grep exits 1 on
# no match and `set -e` would kill the script mid-loop.
scan_line() { adb logcat -d -s ScreenGuard | grep "scan pkg=" | tail -1 || true; }

echo "==> 2. benign text stays clear"
# Start from the launcher so a cover left by an earlier run cannot be mistaken
# for this step's result.
adb shell input keyevent KEYCODE_HOME >/dev/null
sleep 1
adb logcat -c
adb shell am start -a android.settings.SETTINGS >/dev/null
line=""
for _ in $(seq 1 15); do
    line="$(scan_line)"
    [[ -n "$line" ]] && break
    sleep 1
done
[[ -n "$line" ]] || fail "no scan happened in Settings — the service is not seeing other apps"
echo "    $line"
grep -q "action=ALLOW" <<<"$line" || fail "expected ALLOW for the Settings screen, got: $line"

echo "==> 3. dictionary text blocks"
adb logcat -c
# Type into Settings' search field: text from another app's node tree is exactly
# what the AccessibilityTree source represents.
#
# Force-stop first, or `am start` merely brings step 2's existing Settings task
# to the front, the search field never takes focus, and the keystrokes land
# nowhere — a false failure.
adb shell input keyevent KEYCODE_HOME >/dev/null
adb shell am force-stop com.android.settings >/dev/null
adb shell am force-stop com.google.android.settings.intelligence >/dev/null
sleep 1
adb shell am start -a android.settings.APP_SEARCH_SETTINGS >/dev/null 2>&1 || \
    adb shell am start -a android.settings.SETTINGS >/dev/null
sleep 4
adb shell input text "explicit%sact"

# Poll for the verdict: the keystrokes, the app's own re-render, and the
# accessibility event that follows are all asynchronous, so a fixed sleep reads
# the log before the scan that matters has landed.
line=""
for _ in $(seq 1 15); do
    line="$(adb logcat -d -s ScreenGuard | grep "scan pkg=" | grep "action=BLOCK" | tail -1 || true)"
    [[ -n "$line" ]] && break
    sleep 1
done
[[ -n "$line" ]] || fail "expected BLOCK for dictionary text, last saw: $(scan_line)"
echo "    $line"
grep -q "cover=COVER" <<<"$line" || fail "expected cover=COVER, got: $line"
# AccessibilityTree scores at full confidence (multiplier 1.00), so a High
# severity ExactPhrase + TokenSequence hit saturates the clamp.
grep -q "score=100" <<<"$line" || echo "    note: score was not 100 — check the scorer"

echo "==> 4. overlay window present"
adb shell dumpsys window windows | grep -qi "Window{.*$pkg}" \
    || fail "no overlay window found for $pkg"
echo "    ok"

echo "==> 5. cover lifts on a clean screen"
# The complement of step 3, and the more dangerous direction to get wrong: a
# cover that never lifts would pass every check above and leave the device
# unusable.
adb logcat -c
adb shell input keyevent KEYCODE_HOME >/dev/null
cleared=""
for _ in $(seq 1 15); do
    if ! adb shell dumpsys window windows | grep -qi "Window{.*$pkg}"; then
        cleared=1
        break
    fi
    sleep 1
done
[[ -n "$cleared" ]] || fail "overlay still present on the launcher — the cover does not lift"
echo "    $(scan_line)"
echo "    ok"

echo
echo "SMOKE PASS"
