# `image_sandbox_ffiFFI`

The C module of the UniFFI bindings over `packages/image-sandbox-ffi` — the ONNX image classifier
(module 18). Both files it needs — `module.modulemap` and `image_sandbox_ffiFFI.h` — are
**generated and gitignored**; run `scripts/build-ffi.sh` before building. Without them SwiftPM
fails at package-layout resolution:

    error: 'mac-daemon': package has unsupported layout; missing system target module map at
    '.../Sources/image_sandbox_ffiFFI/module.modulemap'

which reads as a broken `Package.swift` rather than as a missing build step. See the sibling
`text_policy_ffiFFI/README.md` for why the name is not a free choice.

One difference from the text-policy crate: this one is built with `--features onnx`, which makes
`ort` fetch a prebuilt ONNX Runtime **at build time**. That is not a runtime network path, so it
does not break the local-first rule — but the first build does need network access. ONNX Runtime
then links statically, so the resulting dylib carries no `libonnxruntime` dependency and nothing
extra has to be embedded in the bundle.
