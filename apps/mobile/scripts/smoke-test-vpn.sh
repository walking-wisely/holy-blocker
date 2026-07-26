#!/usr/bin/env bash
# End-to-end smoke test for the Android network path (NetworkGuardService).
#
# Proves the seam unit tests cannot reach: the generated UniFFI bindings calling
# the real libnet_shield_ffi.so against a real TUN, with the system resolver on
# the other side of it.
#
# What it asserts:
#   1. The guard is restored after a reboot. Android does not re-establish a
#      VpnService by itself, so this is BootReceiver's job — and it was missing
#      in the first version of that class.
#   2. tun0 carries the address NetworkGuard declares.
#   3. A name in the blocklist does not resolve — answered locally, nothing sent.
#   4. A name that is not in it resolves normally, which also proves the device
#      is not black-holed by the single-/32 route.
#   5. Losing VPN consent tears the interface down, records it, and lets the
#      blocked name resolve again — which is what shows the refusal was ours.
#
# **The reboot comes first on purpose, and it is load-bearing.** netd answers a
# cached name without asking any resolver, so a positive result cached before the
# VPN came up survives it coming up: the filter never sees the query and `ping`
# still reports an address. A reboot is the only cache flush available to a
# non-root shell that reliably works — `ndc resolver flushnetdns` fails silently
# here. Running the blocked check on a cold cache is what makes it mean anything,
# and an earlier version of this script failed against a *working* guard for
# exactly this reason.
#
# Usage: smoke-test-vpn.sh   (needs a booted emulator or device on adb)
#
# It writes the protection mode and the blocklist directly through `run-as`,
# which is the emulator equivalent of the user arming the app and choosing a
# list. That needs a debuggable build — i.e. the debug APK, which is what this
# installs.
set -euo pipefail

pkg="com.holyblocker.mobile"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_dir="$(dirname "$here")"
apk="$mobile_dir/app/build/outputs/apk/debug/app-debug.apk"

# Names that really resolve, so a refusal is unambiguously ours. A reserved name
# (RFC 2606) would NXDOMAIN with or without the filter and prove nothing.
blocked="example.com"
allowed="example.org"

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

wait_for_boot() {
    adb wait-for-device
    until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
        sleep 2
    done
}

tun_up() { [[ "$(adb shell ip -o addr show 2>/dev/null | grep -c 'tun0.*inet ')" != "0" ]]; }

# True when the name resolved to an address.
#
# Asserted on the *positive* form — `PING host (1.2.3.4)` — rather than on an
# error string. `ping` has several ways of saying a name did not resolve
# ("unknown host", "bad address", a bare non-zero exit), and matching one of them
# made every other one read as success. That is a test that passes when the guard
# is broken, which is the only kind of wrong that matters here.
lookup_ok() { adb shell "ping -c 1 -W 3 $1" 2>&1 | head -1 | grep -qE "^PING .*\([0-9a-fA-F.:]+\)"; }

lookup_fails() { ! lookup_ok "$1"; }

# Retried: a lookup racing the guard's first network callback can miss while the
# upstream list is still empty, which looks the same as a block.
resolves() {
    for _ in 1 2 3 4 5; do
        lookup_ok "$1" && return 0
        sleep 2
    done
    return 1
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

echo "==> granting the VPN op"
# The emulator equivalent of accepting the consent dialog, which is a system
# window an instrumented test cannot tap.
adb shell appops set "$pkg" ACTIVATE_VPN allow

echo "==> arming protection and installing a blocklist"
arm true
adb shell "run-as $pkg sh -c 'printf \"# smoke test\n$blocked\n\" > files/blocklist.txt'"

# Bring the guard up once before the reboot, so the reboot is testing that it
# comes *back* rather than that it can start at all.
adb shell am start -n "$pkg/.MainActivity" >/dev/null 2>&1
sleep 6
tun_up || fail "tun0 did not come up before the reboot"

echo "==> rebooting (also the only reliable DNS cache flush here)"
adb logcat -c
adb reboot
wait_for_boot
# BootReceiver fires with the rest of the boot broadcasts; give the network
# callback time to hand it a resolver as well.
sleep 30

echo "==> checking the guard came back by itself"
tun_up || fail "tun0 did not return after the reboot — BootReceiver did not restore it"
adb shell ip -o addr show 2>/dev/null | grep -q "10.111.222.1" \
    || fail "tun0 is not carrying NetworkGuard.TUN_ADDRESS"
echo "    tun0 up, restored without the app being opened"

echo "==> checking the blocked name (cold cache)"
lookup_fails "$blocked" || fail "$blocked resolved; it should have been refused locally"
adb logcat -d -s NetworkGuard:V | grep -q "answered a blocked name locally" \
    || fail "no local answer was logged; the refusal may have come from the resolver"
echo "    $blocked refused"

echo "==> checking a permitted name"
# Also the online check: if the route were wider than a single /32 this would
# fail, because there is nothing to forward anything but DNS with.
resolves "$allowed" || fail "$allowed did not resolve; forwarding is broken"
echo "    $allowed resolves"

echo "==> revoking consent"
adb shell appops set "$pkg" ACTIVATE_VPN deny
adb shell am start -n "$pkg/.MainActivity" >/dev/null 2>&1
sleep 5
! tun_up || fail "tun0 survived the consent being revoked"
adb shell "run-as $pkg tail -5 files/tamper-log.tsv" | grep -qE "net_revoked|net_off" \
    || fail "the teardown was not recorded in the tamper log"
# The refusal must have been ours: with the guard gone the same name resolves.
# Retried past the negative-cache TTL left by the check above.
resolves "$blocked" || fail "$blocked still does not resolve with the guard down"
echo "    torn down, recorded, and the block lifted with it"

echo
echo "SMOKE PASS"
