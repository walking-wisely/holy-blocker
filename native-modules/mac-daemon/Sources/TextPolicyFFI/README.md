# `TextPolicyFFI`

The Swift half of the UniFFI bindings over `packages/text-policy-ffi` — `PolicyEngine`, `evaluate`,
`Action`, `SourceKind`, `Verdict`. `generated/text_policy_ffi.swift` is produced by
`scripts/build-ffi.sh` and gitignored, the same treatment `apps/mobile` gives its generated Kotlin.

This target exists so the daemon consumes the one Rust policy engine instead of adding a third
independent implementation of the thresholding rules next to the Rust and the planned C++ ones.
Nothing hand-written belongs here: put Swift-side adaptation (the five-case `Action` to the
three-case `ScanAction`, for instance) in `MacDaemon`, so this directory can be deleted and
regenerated at will.
