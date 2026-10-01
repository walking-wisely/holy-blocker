# Module 0 — the signed bundle, and the TCC identity trap

Part of the [macOS Daemon plan](../plan.md).

**This is a prerequisite of every other Layer 2 module and must be built first.**

TCC identifies a client by its code-signing identity, not by its path. A SwiftPM executable target
produces an unsigned Mach-O; grants made to it are keyed to an ad-hoc identity derived from the
binary's `cdhash`, which changes on **every rebuild**. The practical consequence is that a grant
obtained on Monday stops applying the first time `swift build` runs, and the failure is silent —
`SCShareableContent` simply returns nothing rather than raising a permission error.

**The trap that will actually cost a day, though, is the responsible-process rule.** When a
command-line binary is launched from a terminal, TCC does not attribute the request to the binary —
it attributes it to the *responsible process*, which is Terminal.app or iTerm. So the first run
pops a prompt naming the terminal, the developer grants it, capture starts working, and the
conclusion "permissions are handled" is wrong in three ways at once: the grant belongs to the
terminal, it covers every other tool run from that terminal, and it evaporates the moment the
daemon is launched by `launchd` instead. **Never validate a TCC-gated path by running it from a
shell.** Launch it the way it will ship.

What module 0 therefore has to produce:

- A real `.app` bundle (`HolyBlockerDaemon.app`) with an `Info.plist` carrying a stable
  `CFBundleIdentifier` and the usage-description strings, since a bundle is the only artifact TCC
  will hold a durable grant against.
- A **stable signing identity**. A Developer ID certificate is the correct answer; a self-signed
  certificate in the login keychain is an acceptable development stand-in *provided it is the same
  certificate across rebuilds* — that is the property that matters, not the certificate's
  provenance.
- A `launchd` job (`LaunchDaemon` for the privileged Layer 1 half, `LaunchAgent` for the Layer 2
  half — capture and AppKit need a GUI session and cannot run in the daemon context). This split is
  new and worth stating plainly: **Layer 1 and Layer 2 cannot be the same process.** Layer 1 wants
  root and no session; Layer 2 wants the user's GUI session and must not be root. Expect an
  `XPC`/socket link between them, and expect the existing single `holy-blocker-macd` executable to
  grow a second entry point rather than a second copy of the shared code.

SwiftPM cannot emit an `.app` bundle on its own, so this step adds either a small bundling script
(`scripts/bundle.sh`: assemble the directory layout, write `Info.plist`, `codesign --sign`) or an
Xcode project alongside the package. **Prefer the script** — it keeps the package the source of
truth, keeps `scripts/test.sh` working unchanged, and avoids committing an `.xcodeproj` whose
pbxproj format is hostile to review.

## Built — and one instruction above is wrong

**Done**, including a stable signing identity — see the end of this section and
[signing-identity.md](../signing-identity.md). Shipped: `AppBundle` (layout, `Info.plist` generation, `assemble`),
`LaunchdJob` (both halves), `CodeSigning` (sign, read back, and the `isStable` question),
`scripts/bundle.sh`, and four CLI verbs — `bundle`, `bundle-status`, `launchd-plist`, and `agent`.
The script stayed a script: it builds the binary and asks it to bundle itself, so the tested
`AppBundle` code is the only description of the layout and the shell holds no duplicate plist.

Verified live: `HolyBlockerDaemon.app` assembles, signs, passes `codesign --verify` as *"valid on
disk"* and *"satisfies its Designated Requirement"*, resolves as `com.holyblocker.daemon` through
`Bundle`, and the LaunchAgent **bootstraps into `gui/501`, reaches `state = running`, takes a
permission baseline, and boots out cleanly**. Under launchd the responsible process is the agent
itself rather than a terminal, which is the entire point of the exercise.

Five things the writing turned up:

1. **The instruction to ship "usage-description strings" cannot be followed — those keys do not
   exist for these three capabilities.** The authoritative list is the set of `NS*UsageDescription`
   keys `tccd` itself reads; on macOS 26.5.2 it contains no `NSScreenCaptureUsageDescription`, no
   `NSInputMonitoringUsageDescription`, and nothing for Accessibility. (The widely-repeated advice
   to add them is wrong, and easy to believe because adding a key that nothing reads has no visible
   effect.) Those prompts use fixed system wording and interpolate only the bundle name, so
   **`CFBundleName` carries the entire opportunity to say who is asking.** Extract the real list
   with `strings /System/Library/PrivateFrameworks/TCC.framework/Support/tccd | grep UsageDescription`
   rather than trusting a blog post.
2. **A bare SwiftPM binary is already ad-hoc signed** — Apple silicon will not execute an unsigned
   Mach-O at all. So the problem module 0 solves is *not* an absent signature; it is that the
   ad-hoc identity is derived from the `cdhash` and therefore changes on every build. `unsigned`
   is a state you will rarely actually see.
3. **`Bundle.main.bundleURL` is the containing directory when there is no bundle**, not the
   binary — so passing it to `codesign` fails with "bundle format unrecognized" for exactly the
   un-bundled case a diagnostic is most needed in. `CodeSigning.codePath` picks the right one.
4. **`codesign -dvv` writes its report to stderr.** Reading stdout reports every bundle on the
   machine as unsigned.
5. **Assembly must delete the previous bundle rather than write over it** — a `_CodeSignature`
   directory left from the last build makes `codesign` refuse the new signature.

The `launchd` split is encoded rather than described: the agent carries
`LimitLoadToSessionType = Aqua` and **no** `UserName` key (a TCC grant belongs to the logged-in
user, and root cannot be given one), while the daemon carries `UserName = root` and no session
restriction.

~~**What is still open, and it is a decision, not code:** the bundle signs ad-hoc by default, which
`bundle` and `agent` both warn about at runtime. A stable identity — a Developer ID certificate, or
a self-signed code-signing certificate used consistently — has to exist before any TCC grant is
worth making, and creating one writes to a keychain. Set `HOLY_BLOCKER_SIGNING_IDENTITY` and
`scripts/bundle.sh` uses it. Until then, backlog item 1 stays open.~~ **Done** — a self-signed
`Holy Blocker Dev` code-signing identity now lives in the development machine's login keychain,
`bundle-status` reports `signed(authority: "Holy Blocker Dev")` with `grants survive a rebuild:
true`, and `scripts/bundle.sh` is wired to it via `HOLY_BLOCKER_SIGNING_IDENTITY`. See
[signing-identity.md](../signing-identity.md) for the runbook — where the identity lives, how to
recreate it if lost, and how to rotate it. This is still a development-only identity; a Developer
ID certificate is a separate, later decision needed before shipping to anyone else's machine (see
that page's "Before shipping to anyone else"). Backlog item 1's remaining blocker is now only the
human step: granting Screen Recording / Accessibility to the signed bundle and observing a real
revocation.

---
