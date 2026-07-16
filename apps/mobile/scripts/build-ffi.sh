#!/usr/bin/env bash
# Builds packages/text-policy-ffi for Android and refreshes the generated
# Kotlin bindings.
#
# Two separate outputs, and they have different prerequisites:
#   1. Kotlin bindings  — needs only cargo (generated from the host cdylib).
#   2. libtext_policy_ffi.so per ABI — needs the Android NDK + cargo-ndk.
#
# Prerequisites:
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#   cargo install cargo-ndk
#   sdkmanager --install "ndk;27.2.12479018"   (and export ANDROID_NDK_HOME)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_dir="$(dirname "$here")"
ffi_dir="$mobile_dir/../../packages/text-policy-ffi"

bindings_out="$mobile_dir/app/src/generated/kotlin"
jni_out="$mobile_dir/app/src/main/jniLibs"

# --- 1. Kotlin bindings -----------------------------------------------------
# Generated from the host build; the bindings are platform independent, so this
# does not need the NDK.
echo "==> building host cdylib for binding generation"
cargo build --manifest-path "$ffi_dir/Cargo.toml" --lib

host_lib=""
for candidate in \
    "$ffi_dir/target/debug/libtext_policy_ffi.dylib" \
    "$ffi_dir/target/debug/libtext_policy_ffi.so"; do
    [[ -f "$candidate" ]] && host_lib="$candidate" && break
done
[[ -n "$host_lib" ]] || { echo "no host cdylib found" >&2; exit 1; }

echo "==> generating Kotlin bindings -> $bindings_out"
rm -rf "$bindings_out"
# --features bindgen: the CLI is off by default so `cargo test` builds on the
# toolchain CI pins (its deps need a newer rustc). Run from the crate dir too —
# uniffi-bindgen shells out to `cargo metadata`, which resolves from the working
# directory rather than --manifest-path.
(cd "$ffi_dir" && cargo run --quiet --features bindgen --bin uniffi-bindgen -- \
    generate --library "$host_lib" --language kotlin --no-format --out-dir "$bindings_out")

# --- 2. Android native libraries -------------------------------------------
if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo
    echo "cargo-ndk not found — skipping the .so build."
    echo "Bindings are up to date and unit tests will run, but the app cannot"
    echo "start on a device until the native library is built:"
    echo "  cargo install cargo-ndk && sdkmanager --install 'ndk;27.2.12479018'"
    exit 0
fi

echo "==> building Android native libraries -> $jni_out"
mkdir -p "$jni_out"
# As above, run from the crate dir: cargo-ndk resolves Cargo.toml from the
# working directory, not --manifest-path. $jni_out is absolute, so -o still
# lands in the right place.
(cd "$ffi_dir" && cargo ndk \
    -t arm64-v8a \
    -t armeabi-v7a \
    -t x86_64 \
    -o "$jni_out" \
    build --release)

echo "done"
