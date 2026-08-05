# `ImageSandboxFFI`

The Swift half of the UniFFI bindings over `packages/image-sandbox-ffi` — `ImageGuard`,
`ImageOutcome`, `FramePixelLayout`, `defaultExplicitThreshold()`.
`generated/image_sandbox_ffi.swift` is produced by `scripts/build-ffi.sh` and gitignored, the same
treatment `apps/mobile` gives its generated Kotlin.

The target also carries the linker settings that put `@executable_path/../Frameworks` and the
build-tree `.ffi/lib` on the rpath. They live here rather than on the executable because linker
settings propagate to every product linking the target, which is the only way to reach the
`.xctest` bundle — `DYLD_LIBRARY_PATH` does not work, since SIP strips it across the exec of
Apple's signed `swiftpm-testing-helper`.

`MacDaemon` wraps everything here behind `ImageClassifying` (see `ImageScanner.swift`), so no test
in the package loads this library or needs a model artifact.
