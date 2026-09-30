# `packages/text-policy-ffi` — status

Language: Rust

UniFFI wrapper over text-policy — PolicyEngine + evaluate exposed; Kotlin bindings generated for Android, and Swift bindings for the mac daemon (`native-modules/mac-daemon/scripts/build-ffi.sh`). Two consumers now, so **the crate's API is a cross-platform contract** — changing `Action`, `SourceKind` or `Verdict` breaks both, and the score is a `u32` 0–100 that each caller normalizes itself. No Rust changes were needed for the Swift path
