#!/bin/bash
# Build holy-blocker-macd and wrap it in a signed HolyBlockerDaemon.app.
#
# The bundle exists for one reason: TCC keys a permission grant to a code-signing identity, and a
# bare SwiftPM executable has only an ad-hoc identity derived from its cdhash — which changes on
# every `swift build`, taking every grant with it.
#
# SwiftPM cannot emit an .app, and an .xcodeproj would put a second, unreviewable source of truth
# next to Package.swift. So the assembly lives in `AppBundle`, under test, and this script is only
# the two steps that must happen outside the binary: build it, then ask it to bundle itself.
#
# Signing identity:
#   HOLY_BLOCKER_SIGNING_IDENTITY unset  -> ad-hoc. Runnable, but grants die on the next build.
#   set to a keychain identity name      -> stable. Required before granting anything for real.
#
# A self-signed certificate from Keychain Access ("Certificate Assistant > Create a Certificate",
# type "Code Signing") is an acceptable development identity. What matters is that it is the *same*
# certificate across rebuilds, not where it came from.
set -euo pipefail

cd "$(dirname "$0")/.."

CONFIGURATION="${CONFIGURATION:-debug}"
IDENTITY="${HOLY_BLOCKER_SIGNING_IDENTITY:--}"
OUTPUT="${OUTPUT:-.build}"

# The daemon links libtext_policy_ffi.dylib, which is generated rather than committed. This has to
# run before `swift build`, and the same staged copy is what gets embedded in the bundle below —
# the bundled dylib must be the one the executable was linked against, not another build of it.
./scripts/build-ffi.sh

swift build -c "$CONFIGURATION"

BINARY="$(swift build -c "$CONFIGURATION" --show-bin-path)/holy-blocker-macd"
[[ -x "$BINARY" ]] || { echo "bundle.sh: no binary at $BINARY" >&2; exit 1; }

"$BINARY" bundle "$OUTPUT" "$IDENTITY" "$PWD/.ffi/lib"

echo
echo "next: $OUTPUT/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd bundle-status"
