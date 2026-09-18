#!/usr/bin/env bash
# Builds the UniFFI crates the daemon links against and refreshes the generated Swift bindings.
#
# Crates:
#   packages/text-policy-ffi    -> libtext_policy_ffi.dylib     (the AX text path, module 16)
#   packages/image-sandbox-ffi  -> libimage_sandbox_ffi.dylib   (the screen image path, module 18)
#
# Modelled on apps/mobile/scripts/build-ffi.sh, with two differences: `--language swift` instead of
# kotlin, and real dylibs the daemon links against and embeds in Contents/Frameworks rather than a
# per-ABI .so the JVM dlopens by name.
#
# Outputs, none of which are committed (see .gitignore):
#   Sources/<lib>FFI/{module.modulemap,<lib>FFI.h}   the C module, one target per crate
#   Sources/<Target>/generated/<lib>.swift           the Swift API, one target per crate
#   .ffi/lib/lib<lib>.dylib                          the libraries to link and embed
#
# Prerequisites: rustup target add aarch64-apple-darwin (installed by default on Apple silicon).
#
# image-sandbox-ffi is built with --features onnx, which makes `ort` fetch a prebuilt ONNX Runtime
# at build time. That is a build-time download, not a runtime network path, so it does not break the
# local-first rule — but it does mean the first build of that crate needs network access.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
package_dir="$(dirname "$here")"
packages_dir="$package_dir/../../packages"

# crate directory : cdylib base name (cargo replaces - with _ in [lib].name) : SwiftPM target :
# extra cargo features. The cdylib name is not a free choice downstream — the generated Swift
# contains a literal `import <lib>FFI`, and a SwiftPM systemLibrary module name *is* its target
# name, so the C target must be named exactly `<lib>FFI`.
crates=(
    "text-policy-ffi:text_policy_ffi:TextPolicyFFI:"
    "image-sandbox-ffi:image_sandbox_ffi:ImageSandboxFFI:onnx"
)

profile="${PROFILE:-release}"
staging="$package_dir/.ffi"
lib_out="$staging/lib"

# Explicit --target so the output path is predictable rather than depending on whether cargo is
# cross-compiling; on an Apple silicon host this is also the host triple.
target_triple="aarch64-apple-darwin"

mkdir -p "$lib_out"

for entry in "${crates[@]}"; do
    IFS=':' read -r crate_name lib_name swift_target features <<<"$entry"
    crate_dir="$packages_dir/$crate_name"
    dylib="lib${lib_name}.dylib"
    c_module="${lib_name}FFI"
    swift_out="$package_dir/Sources/$swift_target/generated"
    c_out="$package_dir/Sources/$c_module"

    # --- 1. the dylib -------------------------------------------------------
    echo "==> building $dylib ($profile, $target_triple${features:+, features: $features})"
    cargo build --manifest-path "$crate_dir/Cargo.toml" --lib --target "$target_triple" \
        ${features:+--features "$features"} \
        $([[ "$profile" == "release" ]] && echo --release)

    built="$crate_dir/target/$target_triple/$profile/$dylib"
    [[ -f "$built" ]] || { echo "build-ffi.sh: no dylib at $built" >&2; exit 1; }

    cp "$built" "$lib_out/$dylib"

    # cargo stamps the dylib's LC_ID_DYLIB with the *absolute path of its own target directory*. An
    # executable linked against it records that path verbatim, so the app keeps loading the copy in
    # the crate's target/ — and fails outright on any machine that has no such directory. Rewriting
    # the id to @rpath is what makes Contents/Frameworks work at all.
    install_name_tool -id "@rpath/$dylib" "$lib_out/$dylib"
    # The id rewrite invalidates the ad-hoc signature cargo's linker applied (Apple silicon refuses
    # an unsigned Mach-O), so re-sign. The bundle re-signs this with the real identity later.
    codesign --force --sign - "$lib_out/$dylib"

    # --- 2. the bindings ----------------------------------------------------
    # uniffi writes the .swift, the .h and the modulemap into one directory; SwiftPM needs the
    # Swift in one target and the C in another, so generate to a staging dir and split.
    generated="$staging/generated/$lib_name"
    rm -rf "$generated" "$swift_out"
    mkdir -p "$generated" "$swift_out" "$c_out"

    echo "==> generating Swift bindings for $crate_name"
    # --features bindgen: the CLI is off by default so `cargo test` builds on the toolchain CI
    # pins. Run from the crate dir — uniffi-bindgen shells out to `cargo metadata`, which resolves
    # from the working directory rather than --manifest-path.
    (cd "$crate_dir" && cargo run --quiet --features bindgen --bin uniffi-bindgen -- \
        generate --library "$lib_out/$dylib" --language swift --no-format --out-dir "$generated")

    cp "$generated/$lib_name.swift" "$swift_out/"
    cp "$generated/$c_module.h" "$c_out/"
    # SwiftPM looks for `module.modulemap` by that exact name. Its contents are copied unedited:
    # the module it declares is `<lib>FFI`, which is why the SwiftPM target has that name too.
    cp "$generated/$c_module.modulemap" "$c_out/module.modulemap"
done

echo "done"
