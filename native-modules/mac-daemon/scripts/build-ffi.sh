#!/usr/bin/env bash
# Builds packages/text-policy-ffi for macOS and refreshes the generated Swift bindings.
#
# Modelled on apps/mobile/scripts/build-ffi.sh, with three differences: one crate instead of two,
# `--language swift` instead of kotlin, and a real dylib the daemon links against rather than a
# per-ABI .so the JVM dlopens by name.
#
# Outputs, none of which are committed (see .gitignore):
#   Sources/text_policy_ffiFFI/{module.modulemap,text_policy_ffiFFI.h}  the C module
#   Sources/TextPolicyFFI/generated/text_policy_ffi.swift               the Swift API
#   .ffi/lib/libtext_policy_ffi.dylib                                   the library to link
#
# Prerequisites: rustup target add aarch64-apple-darwin (installed by default on Apple silicon).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
package_dir="$(dirname "$here")"
crate_dir="$package_dir/../../packages/text-policy-ffi"

lib_name="text_policy_ffi"        # Cargo replaces - with _ in [lib].name
dylib="lib${lib_name}.dylib"
c_module="${lib_name}FFI"         # fixed by uniffi: the generated .swift does `import text_policy_ffiFFI`

profile="${PROFILE:-release}"
staging="$package_dir/.ffi"
swift_out="$package_dir/Sources/TextPolicyFFI/generated"
c_out="$package_dir/Sources/$c_module"
lib_out="$staging/lib"

# --- 1. the dylib -----------------------------------------------------------
# Explicit --target so the output path is predictable rather than depending on whether cargo is
# cross-compiling; on an Apple silicon host this is also the host triple.
target_triple="aarch64-apple-darwin"
echo "==> building $dylib ($profile, $target_triple)"
cargo build --manifest-path "$crate_dir/Cargo.toml" --lib --target "$target_triple" \
    $([[ "$profile" == "release" ]] && echo --release)

built="$crate_dir/target/$target_triple/$profile/$dylib"
[[ -f "$built" ]] || { echo "build-ffi.sh: no dylib at $built" >&2; exit 1; }

mkdir -p "$lib_out"
cp "$built" "$lib_out/$dylib"

# cargo stamps the dylib's LC_ID_DYLIB with the *absolute path of its own target directory*. An
# executable linked against it records that path verbatim, so the app keeps loading the copy in
# packages/text-policy-ffi/target — and fails outright on any machine that has no such directory.
# Rewriting the id to @rpath is what makes Contents/Frameworks work at all.
install_name_tool -id "@rpath/$dylib" "$lib_out/$dylib"
# The id rewrite invalidates the ad-hoc signature cargo's linker applied (Apple silicon refuses an
# unsigned Mach-O), so re-sign. The bundle re-signs this with the real identity later.
codesign --force --sign - "$lib_out/$dylib"

# --- 2. the bindings --------------------------------------------------------
# uniffi writes the .swift, the .h and the modulemap into one directory; SwiftPM needs the Swift in
# one target and the C in another, so generate to a staging dir and split.
generated="$staging/generated"
rm -rf "$generated" "$swift_out"
mkdir -p "$generated" "$swift_out" "$c_out"

echo "==> generating Swift bindings"
# --features bindgen: the CLI is off by default so `cargo test` builds on the toolchain CI pins.
# Run from the crate dir — uniffi-bindgen shells out to `cargo metadata`, which resolves from the
# working directory rather than --manifest-path.
(cd "$crate_dir" && cargo run --quiet --features bindgen --bin uniffi-bindgen -- \
    generate --library "$lib_out/$dylib" --language swift --no-format --out-dir "$generated")

cp "$generated/$lib_name.swift" "$swift_out/"
cp "$generated/$c_module.h" "$c_out/"
# SwiftPM looks for `module.modulemap` by that exact name. Its contents are copied unedited: the
# module it declares is `text_policy_ffiFFI`, which is why the SwiftPM target has that name too.
cp "$generated/$c_module.modulemap" "$c_out/module.modulemap"

echo "done"
