---
name: android-e2e
description: Build, install and verify apps/mobile end to end on a local Android emulator — SDK discovery, FFI libs, headless boot, the three smoke tests, teardown. Use when asked to run the Android app, verify an apps/mobile change on a device, run the smoke tests, or take a Gradle/AGP/dependency change to "works on the emulator". Triggers on "e2e", "emulator", "smoke test", "run on device", "adb".
---

# Android end-to-end on a local emulator

Everything here is machine-agnostic: nothing below assumes a username, an SDK path, an AVD
name or a hardware model. Discover, do not hardcode. Read
`docs/components/mobile/gotchas.md` before touching an Android API — most surprises are
already written down there.

## 0. Credentials

None. This path is local-only: no cloud calls, no signing, no store. If a step seems to
want a secret, stop — see `docs/engineering/secrets-for-agents.md`.

## 1. Resolve the environment (every new shell)

```bash
. .claude/skills/android-e2e/env.sh
```

Shell state does not persist between tool calls, so source it in the same command that uses
it. It prints the SDK, NDK, available AVDs and installed Rust Android targets. Two traps it
absorbs: a package-manager `sdkmanager` may keep a *second* SDK root, so the NDK or a system
image can be "missing" from the one Gradle sees; and `adb`/`emulator` are not on `PATH` by
default.

## 2. One-time per machine

- `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`
  (`build-ffi.sh` builds all three ABIs; missing ones fail with `can't find crate for core`).
- `cargo-ndk` installed, and an NDK reachable (`env.sh` reports it).
- An AVD. If `emulator -list-avds` shows none usable, create one matching the host CPU
  architecture, using a **`google_apis`** image (not `google_apis_playstore` — the smoke
  tests write `enabled_accessibility_services` directly, which Play images refuse), API 36:

  ```bash
  sdkmanager "system-images;android-36;google_apis;arm64-v8a"   # x86_64 on Intel hosts
  avdmanager create avd -n holyblocker-test -k "system-images;android-36;google_apis;arm64-v8a"
  ```

## 3. Build

From `apps/mobile`, in the worktree you are working in (each worktree has its own gitignored
`jniLibs/` and `src/generated/`, so a fresh worktree has neither):

```bash
./scripts/build-ffi.sh
./gradlew :app:assembleDebug :app:testDebugUnitTest
```

Check the unit tests actually ran: count `tests=` in
`app/build/test-results/testDebugUnitTest/*.xml`. A green exit with zero tests has happened
in this repo's toolchains.

## 4. Boot

```bash
emulator -avd <name> -no-window -no-audio -no-snapshot -no-boot-anim >"$TMPDIR/emu.log" 2>&1 &
```

Run it with `run_in_background`. Then poll `adb shell getprop sys.boot_completed` until it
prints `1` — `adb wait-for-device` returns while the system is still half-booted. macOS has
no `timeout`; bound the poll with a counted `for` loop.

If several worktrees run emulators at once, give each its own console port
(`emulator ... -port <even number in 5554–5584>`) and export `ANDROID_SERIAL=emulator-<port>`
for that worktree's commands; the scripts call plain `adb`, which honours it. This
isolation is standard emulator/adb behaviour but has not been exercised in this repo.

## 5. Verify

```bash
./scripts/smoke-test.sh          # text path: service connects, ALLOW/BLOCK, overlay
./scripts/smoke-test-vpn.sh      # DNS path: blocked name refused, permitted name resolves
./scripts/smoke-test-capture.sh  # MediaProjection path: frames gated, revoke noticed
```

Each prints `SMOKE PASS` or `SMOKE FAIL: <why>`. Run all three; a change to AGP, Gradle,
manifest, or permissions can break any one without touching the others. Pipe through `tail`
but do not trust the pipeline's exit status — read the last line.

`smoke-test.sh` prints `note: score was not 100` on a benign pass; that is existing
behaviour, not a regression.

## 6. Teardown

```bash
adb emu kill
```

Leave the AVD; never `-wipe-data` unless a test needs a cold install. Do not leave an
emulator running between unrelated tasks.

## What this proves, and what it does not

It proves the build, the UniFFI seam, the three services and the guard on one emulator image.
It does not prove vendor-skin behaviour (Recents, OEM settings screens — see `backlog.md`),
the image classification path (not built), or anything on a real network. Report it as
"verified on an emulator", with the API level, never as "works on Android".
