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
#
# The suite also links libtext_policy_ffi.dylib, whose bindings and binary are generated rather than
# committed, so the FFI build runs first. Set HOLY_BLOCKER_SKIP_FFI=1 to skip it when iterating on
# Swift alone and the bindings are known to be current.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${HOLY_BLOCKER_SKIP_FFI:-0}" != "1" ]]; then
    ./scripts/build-ffi.sh
fi

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

# Note there is deliberately no DYLD_LIBRARY_PATH here for libtext_policy_ffi.dylib. It does not
# work: `swift test` execs Apple's signed swiftpm-testing-helper, SIP strips every DYLD_* variable
# across that exec, and the library is left unfound with the variable absent from dyld's search
# list entirely. The build-tree rpath in Package.swift is what actually resolves it.
#
# ${flags[@]+...} guards the expansion: macOS ships bash 3.2, where an empty array under `set -u`
# is an unbound-variable error rather than an empty expansion.
exec swift test ${flags[@]+"${flags[@]}"} "$@"
