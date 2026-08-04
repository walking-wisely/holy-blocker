# `text_policy_ffiFFI`

The C module of the UniFFI bindings over `packages/text-policy-ffi`. Both files it needs —
`module.modulemap` and `text_policy_ffiFFI.h` — are **generated and gitignored**; run
`scripts/build-ffi.sh` before building. Without them SwiftPM fails at package-layout resolution:

    error: 'mac-daemon': package has unsupported layout; missing system target module map at
    '.../Sources/text_policy_ffiFFI/module.modulemap'

which reads as a broken `Package.swift` rather than as a missing build step.

The odd name is not a choice. The generated `text_policy_ffi.swift` contains a literal
`import text_policy_ffiFFI`, and a SwiftPM `systemLibrary` target's module name is its target name,
so the target has to match what UniFFI emits. Renaming it means patching generated code on every
build.
