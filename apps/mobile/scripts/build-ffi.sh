#!/usr/bin/env bash
# Builds the UniFFI crates this app links against and refreshes the generated
# Kotlin bindings.
#
# Crates:
#   packages/text-policy-ffi  -> libtext_policy_ffi.so   (the text path)
#   packages/net-shield-ffi   -> libnet_shield_ffi.so    (the VPN DNS path)
#
# Two separate outputs, and they have different prerequisites:
#   1. Kotlin bindings  — needs only cargo (generated from the host cdylib).
#   2. lib*.so per ABI  — needs the Android NDK + cargo-ndk.
#
# Prerequisites:
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#   cargo install cargo-ndk
#   sdkmanager --install "ndk;27.2.12479018"   (and export ANDROID_NDK_HOME)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_dir="$(dirname "$here")"
packages_dir="$mobile_dir/../../packages"

# Crate directory name : cdylib base name (Cargo replaces - with _ in [lib].name)
crates=(
    "text-policy-ffi:text_policy_ffi"
    "net-shield-ffi:net_shield_ffi"
)

bindings_out="$mobile_dir/app/src/generated/kotlin"
jni_out="$mobile_dir/app/src/main/jniLibs"

# --- 1. Kotlin bindings -----------------------------------------------------
# Generated from the host build; the bindings are platform independent, so this
# does not need the NDK. Cleared once, before the loop — each crate generates
# into its own `uniffi/<namespace>` subtree, so clearing per crate would delete
# the bindings the previous iteration just wrote.
rm -rf "$bindings_out"

for entry in "${crates[@]}"; do
    crate_dir="$packages_dir/${entry%%:*}"
    lib_name="${entry##*:}"

    echo "==> building host cdylib for binding generation: ${entry%%:*}"
    cargo build --manifest-path "$crate_dir/Cargo.toml" --lib

    host_lib=""
    for candidate in \
        "$crate_dir/target/debug/lib${lib_name}.dylib" \
        "$crate_dir/target/debug/lib${lib_name}.so"; do
        [[ -f "$candidate" ]] && host_lib="$candidate" && break
    done
    [[ -n "$host_lib" ]] || { echo "no host cdylib found for ${entry%%:*}" >&2; exit 1; }

    echo "==> generating Kotlin bindings -> $bindings_out"
    # --features bindgen: the CLI is off by default so `cargo test` builds on the
    # toolchain CI pins (its deps need a newer rustc). Run from the crate dir too —
    # uniffi-bindgen shells out to `cargo metadata`, which resolves from the working
    # directory rather than --manifest-path.
    (cd "$crate_dir" && cargo run --quiet --features bindgen --bin uniffi-bindgen -- \
        generate --library "$host_lib" --language kotlin --no-format --out-dir "$bindings_out")
done

# --- 2. Android native libraries -------------------------------------------
if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo
    echo "cargo-ndk not found — skipping the .so build."
    echo "Bindings are up to date and unit tests will run, but the app cannot"
    echo "start on a device until the native libraries are built:"
    echo "  cargo install cargo-ndk && sdkmanager --install 'ndk;27.2.12479018'"
    exit 0
fi

for entry in "${crates[@]}"; do
    crate_dir="$packages_dir/${entry%%:*}"
    echo "==> building Android native libraries -> $jni_out: ${entry%%:*}"
    mkdir -p "$jni_out"
    # As above, run from the crate dir: cargo-ndk resolves Cargo.toml from the
    # working directory, not --manifest-path. $jni_out is absolute, so -o still
    # lands in the right place.
    (cd "$crate_dir" && cargo ndk \
        -t arm64-v8a \
        -t armeabi-v7a \
        -t x86_64 \
        -o "$jni_out" \
        build --release)
done

echo "done"
