#!/bin/bash
# Run the mac-daemon test suite.
#
# swift-testing ships inside Xcode, but a Command Line Tools-only install puts Testing.framework
# somewhere SwiftPM does not search, so a bare `swift test` fails to compile it and then fails to
# dlopen it at runtime. This script adds the framework search path and both rpaths when the active
# toolchain is the Command Line Tools, and does nothing extra otherwise.
#
# These flags cannot live in Package.swift: `unsafeFlags` on a test target makes SwiftPM silently
# skip test discovery, so `swift test` builds, prints nothing, and exits 0.
set -euo pipefail

cd "$(dirname "$0")/.."

CLT_FRAMEWORKS="/Library/Developer/CommandLineTools/Library/Developer/Frameworks"
CLT_LIB="/Library/Developer/CommandLineTools/Library/Developer/usr/lib"

flags=()
if [[ "$(xcode-select -p)" == /Library/Developer/CommandLineTools* ]] \
    && [[ -d "$CLT_FRAMEWORKS/Testing.framework" ]]; then
    flags=(
        -Xswiftc -F -Xswiftc "$CLT_FRAMEWORKS"
        -Xlinker -F -Xlinker "$CLT_FRAMEWORKS"
        -Xlinker -rpath -Xlinker "$CLT_FRAMEWORKS"
        -Xlinker -rpath -Xlinker "$CLT_LIB"
    )
fi

# ${flags[@]+...} guards the expansion: macOS ships bash 3.2, where an empty array under `set -u`
# is an unbound-variable error rather than an empty expansion.
exec swift test ${flags[@]+"${flags[@]}"} "$@"
