#!/usr/bin/env bash
# End-to-end smoke test for custom-app enforcement.
#
# What it asserts:
#   1. A chosen app is sent home shortly after it is opened.
#   2. A cover window appears and lifts once the app has gone.
#   3. An unchosen app is left in the foreground.
#   4. Enforcement stops when protection is disarmed.
#
# It also reports, without asserting, whether the chosen app's process
# survives (the home action backgrounds an app, it does not stop it) and how
# long the app was foreground before it was sent home.
#
# The list is seeded through `run-as` with the app process down: the panel that
# writes it does not exist yet, and the service reads the list when it connects.
#
# Usage: smoke-test-custom-app.sh   (needs a booted emulator or device on adb)
set -euo pipefail

pkg="com.holyblocker.mobile"
svc="$pkg/$pkg.ScreenGuardService"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
apk="$(dirname "$here")/app/build/outputs/apk/debug/app-debug.apk"

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

[[ -f "$apk" ]] || fail "no APK at $apk — run ./gradlew :app:assembleDebug"

adb wait-for-device
until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
    sleep 2
done

installed() { adb shell pm list packages "$1" | tr -d '\r' | grep -qx "package:$1"; }

chosen=""
for candidate in com.google.android.deskclock com.android.deskclock com.google.android.calculator com.android.calculator2; do
    if installed "$candidate"; then chosen="$candidate"; break; fi
done
[[ -n "$chosen" ]] || fail "no clock or calculator app on this image to use as the chosen app"

unchosen=""
for candidate in com.android.contacts com.google.android.contacts com.android.documentsui; do
    if installed "$candidate"; then unchosen="$candidate"; break; fi
done
[[ -n "$unchosen" ]] || fail "no unchosen app available"

launch() {
    local component
    component="$(adb shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER "$1" | tr -d '\r' | tail -1)"
    adb shell am start -n "$component" >/dev/null
}

foreground() {
    adb shell dumpsys activity activities | tr -d '\r' | grep -m1 "topResumedActivity" || true
}

seed() {
    adb shell am force-stop "$pkg"
    adb shell "run-as $pkg sh -c 'cat > shared_prefs/protection_mode.xml' <<'EOF'
<?xml version='1.0' encoding='utf-8' standalone='yes' ?>
<map>
    <boolean name=\"armed\" value=\"$1\" />
</map>
EOF"
    adb shell "run-as $pkg sh -c 'printf \"app\\t$chosen\\n\" > files/custom_apps.txt'"
}

enable_service() {
    adb logcat -c
    for _ in $(seq 1 10); do
        adb shell settings put secure enabled_accessibility_services "$svc" >/dev/null
        adb shell settings put secure accessibility_enabled 1 >/dev/null
        sleep 2
        [[ "$(adb shell settings get secure enabled_accessibility_services | tr -d '\r')" == "$svc" ]] && break
    done
    for _ in $(seq 1 15); do
        adb logcat -d -s ScreenGuard | grep -q "screen guard connected" && return 0
        sleep 2
    done
    fail "service never connected"
}

echo "==> installing"
adb install -r -g "$apk" >/dev/null || fail "install failed"
adb shell settings delete secure enabled_accessibility_services >/dev/null
adb shell input keyevent KEYCODE_HOME >/dev/null

echo "==> arming protection and listing $chosen"
seed true
enable_service

echo "==> 1. the chosen app is sent home, and 2. a cover appears and lifts"
adb shell input keyevent KEYCODE_HOME >/dev/null
adb shell am force-stop "$chosen"
component="$(adb shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER "$chosen" | tr -d '\r' | tail -1)"
trace="$(adb shell "(am start -n $component >/dev/null &); for i in \$(seq 1 60); do o=\$(dumpsys window windows | grep -c 'Window{.*$pkg'); t=\$(dumpsys activity activities | grep -m1 topResumedActivity); case \"\$t\" in *$chosen*) f=chosen;; *) f=other;; esac; echo \"\$f \$o\"; done" 2>/dev/null | tr -d '\r')"
first_chosen="$(grep -n '^chosen' <<<"$trace" | head -1 | cut -d: -f1)"
last_chosen="$(grep -n '^chosen' <<<"$trace" | tail -1 | cut -d: -f1)"
first_cover="$(grep -n ' [1-9]' <<<"$trace" | head -1 | cut -d: -f1)"
[[ -n "$first_chosen" ]] || fail "chosen app was never foreground"
[[ "$(tail -1 <<<"$trace")" == "other 0" ]] || fail "chosen app still foreground or cover still up at the end: $(tail -1 <<<"$trace")"
[[ "$last_chosen" -lt 60 ]] || fail "chosen app was never sent home"
[[ -n "$first_cover" ]] || fail "no cover window was ever seen"
echo "    ok: app foreground samples $first_chosen-$last_chosen, first cover sample $first_cover, cover gone at the end"
echo "    (samples are one dumpsys pair each; content showed for $((first_cover - first_chosen)) samples before the cover)"

echo "==> observation: does the chosen app's process survive?"
if adb shell pidof "$chosen" >/dev/null 2>&1; then
    echo "    process still running: send-home backgrounds the app, it does not stop it"
else
    echo "    process gone"
fi

echo "==> 3. an unchosen app is untouched"
launch "$unchosen"
sleep 4
fg="$(foreground)"
[[ "$fg" == *"$unchosen"* ]] || fail "unchosen app was not left in the foreground: $fg"
echo "    ok"

echo "==> 4. disarmed protection stops enforcement"
adb shell input keyevent KEYCODE_HOME >/dev/null
adb shell settings delete secure enabled_accessibility_services >/dev/null
sleep 2
seed false
enable_service
launch "$chosen"
sleep 5
fg="$(foreground)"
[[ "$fg" == *"$chosen"* ]] || fail "chosen app was sent home while protection was off: $fg"
echo "    ok"

adb shell input keyevent KEYCODE_HOME >/dev/null
echo
echo "SMOKE PASS"
