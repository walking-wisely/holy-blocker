# 16. `TextPolicyFFI` — the Rust policy engine over UniFFI. **Done.**

Part of the [macOS Daemon plan](../plan.md).

```
scripts/build-ffi.sh
Sources/text_policy_ffiFFI/     generated C module (gitignored)
Sources/TextPolicyFFI/generated/ generated Swift bindings (gitignored)
.ffi/lib/libtext_policy_ffi.dylib
```

Resolves module 9's "where the mode→action decision should live" question: the daemon consumes
`packages/text-policy-ffi`'s `PolicyEngine`/`evaluate` rather than adding a third independent
implementation of the thresholding rules next to the Rust and the planned C++ ones. No Rust changes
were needed — `SourceKind::AccessibilityTree` and the `u32` score already exist.

`scripts/build-ffi.sh` is modelled on `apps/mobile/scripts/build-ffi.sh`: one crate, `--language
swift`, run from the crate directory because `uniffi-bindgen` shells out to `cargo metadata`, which
resolves from the working directory rather than `--manifest-path`. `scripts/test.sh` and
`scripts/bundle.sh` both call it first; nothing generated is committed, exactly as `apps/mobile`
treats its generated Kotlin.

Verified live: the bundle assembles with the dylib in `Contents/Frameworks`, `codesign --verify
--deep --strict` validates the nested dylib and reports the bundle satisfies its Designated
Requirement, and the bundled binary still runs with `.ffi/lib` **moved away entirely** — proving the
load comes from inside the bundle rather than from the build tree.

## Five findings, two of which contradict the spec above

1. **The C module's name is not a free choice.** The generated `text_policy_ffi.swift` contains a
   literal `import text_policy_ffiFFI`, and a SwiftPM `systemLibrary` target's module name *is* its
   target name — so the target is called `text_policy_ffiFFI`, not the `CTextPolicyFFI` this plan
   asked for. Renaming it means patching generated code on every build.
2. **`DYLD_LIBRARY_PATH` does not work for `swift test`**, so the instruction above to export it is
   wrong. `swift test` execs Apple's signed `swiftpm-testing-helper`, and SIP strips every `DYLD_*`
   variable across that exec; the variable never reaches the process that `dlopen`s the test bundle,
   and it is absent from dyld's search list in the failure output. What actually works is an rpath —
   and it must be declared on the **`TextPolicyFFI` target**, not the executable target, because
   linker settings propagate to every product that links the target and that is the only way to
   reach `MacDaemonPackageTests.xctest` without putting `unsafeFlags` on the test target.
3. **`unsafeFlags` on a *non-test* target does not break test discovery.** The `Package.swift`
   warning is specific to test targets; the FFI target carries `-L` and two `-rpath` flags and all
   250 tests are still found and run. Worth knowing, because the obvious reading of that warning is
   that `unsafeFlags` is banned package-wide.
4. **cargo stamps the dylib's `LC_ID_DYLIB` with the absolute path of its own target directory.**
   An executable linked against it records that path verbatim, so the app keeps loading the copy
   under `packages/text-policy-ffi/target` and fails outright on any machine without that
   directory — a bug that is invisible on the build machine. `build-ffi.sh` rewrites the id to
   `@rpath/libtext_policy_ffi.dylib`. That rewrite then invalidates the ad-hoc signature cargo's
   linker applied (Apple silicon refuses an unsigned Mach-O), so the script re-signs immediately.
5. **An embedded dylib must be signed before the bundle**, because signing the bundle seals whatever
   its contents are at that moment. In the other order the seal describes an unsigned dylib while
   the dylib is signed, and the mismatch is refused at launch rather than at build time.
   `CodeSigning.sign(bundle:identity:nestedCode:)` takes them in that order explicitly; `--deep`
   would do it in one call and is documented by Apple as not to be used, since it signs whatever it
   happens to find rather than what was meant to ship.

`AppBundle` gained the concept it was missing: `BundleLayout.frameworks`, an optional `libraries:`
argument to `assemble`, `AppBundle.embeddedLibraryNames` (kept in Swift rather than in `bundle.sh`
so the name the linker resolves and the name the bundler copies cannot drift apart), and
`AppBundle.nestedCode(in:)`. The `bundle` verb takes an optional third argument, the directory
`build-ffi.sh` staged the dylib in, and warns rather than failing when it is absent — a bundle with
no dylib assembles and signs cleanly and then dies at the first policy call, so the warning is the
only thing between that and a confusing launch failure.

One consequence worth stating plainly: **a fresh checkout cannot build until `scripts/build-ffi.sh`
has run.** SwiftPM fails at package-layout resolution with `missing system target module map at
.../Sources/text_policy_ffiFFI/module.modulemap`; a committed `README.md` in that directory explains
the fix, since the message reads like a broken manifest rather than a missing build step.

---
