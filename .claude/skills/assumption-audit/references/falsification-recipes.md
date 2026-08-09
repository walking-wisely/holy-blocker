# Falsification recipes

One command per claim. Grouped by the surface the claim lives on. Every recipe here exists because
this repository needed it, or because a related claim in the same family turned out to be false.

The case log at the end is the argument for the whole skill: each entry is a claim that was
believed, built on, and then measured.

---

## DNS and resolvers

`packages/domain-blocklist`, `packages/net-shield`, `apps/mobile`'s `NetworkGuardService`.

| Claim shape | Falsifier |
|---|---|
| A TLD supports authenticated denial of existence (`AD=1` on NXDOMAIN) | `dig +dnssec nonexistent-$RANDOM.com @1.1.1.1 \| grep -E '^;; flags'` — look for `ad`. Repeat per TLD; do **not** generalise from one. |
| A zone returns NXDOMAIN for a nonexistent subdomain | `dig nonexistent-$RANDOM.example.com @1.1.1.1 \| grep -E 'status:'` — Cloudflare-hosted zones return `NOERROR` with no answer (compact denial / "black lies"), not `NXDOMAIN`. |
| A resolver is not filtering the swept category | Resolve a known-live in-category control. If the repo cannot hold such a control (it cannot — see the content-fixture rule in `CLAUDE.md`), the canary is structurally blind and the plan must say so. |
| A resolver returns the same answer as another | Query at least three independent resolvers (`1.1.1.1`, `8.8.8.8`, `9.9.9.9`) and diff. A single-resolver measurement is a measurement of that resolver. |
| Extended DNS Errors are or are not present | `dig +dnssec blocked.example @1.1.1.3 \| grep -i 'EDE'` (RFC 8914). Filtering resolvers frequently self-declare with EDE 15/16/17. |
| A response arrives only from the queried resolver | Not a DNS question — a socket question. `connect()` the socket, then verify the transaction ID and question section. An unconnected `DatagramSocket` accepts a datagram from any source. |

Rate limits and ToS are also claims: check the published policy for Tranco, the list sources, and
any API before building a sweep cadence around an assumed one.

## macOS: TCC, signing, capture

`native-modules/mac-daemon`.

| Claim shape | Falsifier |
|---|---|
| An `Info.plist` usage-description key exists for a capability | `strings /System/Library/PrivateFrameworks/TCC.framework/Support/tccd \| grep UsageDescription` — the shipped binary is the authority. Blog posts and older docs list keys that do not exist. |
| A grant belongs to our process | Never test from a shell. A CLI launched from a terminal has its grant attributed to the terminal, so it appears to work, covers every other tool run from that shell, and evaporates under `launchd`. Run under `launchd` or the measurement is void. |
| A bundle is signed the way we think | `codesign -dvv <bundle> 2>&1` — **`-dvv` writes to stderr**; without the redirect every bundle reads as unsigned. Add `codesign --verify --deep --strict`. |
| A grant survives a rebuild | Replace the bundle, then re-run the process's own preflight check (`AXIsProcessTrusted()`, `CGPreflightScreenCaptureAccess()`). Reading System Settings is not a falsifier — a hand-added TCC entry can read ON while the running process is untrusted. |
| A capture stream delivers a given pixel format | Set the format explicitly and log `CVPixelBufferGetPixelFormatType` plus `bytesPerRow` against `width * 4`. Never rely on the default. |
| A display dimension is in pixels | `CGDisplayCopyDisplayMode(...).pixelWidth` vs `SCDisplay.width`. `SCDisplay.width` is in points. |
| A system integrity or account state holds | `csrutil status` (has a third answer: `unknown (Custom Configuration)`), `dscl . -list /Users UniqueID` filtered to UID ≥ 500, `dscl . -read /Groups/admin`. |
| An accessibility opt-in attribute is honoured | Write it and check the returned `AXError`. Chrome 151 returns `kAXErrorAttributeUnsupported` for `AXManualAccessibility`; Electron apps accept the write and change nothing. |

## Android

`apps/mobile`.

| Claim shape | Falsifier |
|---|---|
| A manifest permission is present | `adb shell dumpsys package <pkg> \| grep -A40 'requested permissions'`. Missing `INTERNET` and missing `ACCESS_NETWORK_STATE` each produced a failure that looked like success. |
| A guard is filtering, not just failing | Assert **both** directions: a blocked name is refused *and* a permitted name resolves. Assert on positive output (`ping`'s success line), never on the absence of an error string. |
| A resolver answer came from the resolver | `ndc resolver flushnetdns` fails silently for a non-root shell and netd answers from cache. Reboot for a cold cache. |
| A service survives a lifecycle event | Test the real event: force-stop, reboot, `adb` disable, guest account. `ACTION_BOOT_COMPLETED` is also delivered when the app leaves the force-stopped state, so a boot receiver can never be what identifies a boot. |
| An accessibility node tree is visible | Harvest it. A screen can return zero rows for reasons that have nothing to do with window selection (`accessibilityDataSensitive` needs `android:isAccessibilityTool="true"`). |
| A behaviour reproduces on an emulator | Many OEM behaviours do not (One UI/HyperOS recents-kill). If it needs hardware, mark unverifiable rather than mitigating a claim you cannot observe. |

## Models, ONNX, and thresholds

`packages/image-sandbox`, `packages/classifier-head`, `machine-learning`.

| Claim shape | Falsifier |
|---|---|
| A runtime builds for a target triple | `cargo build --target <triple>` for **every** shipped ABI, not one. `ort` has no prebuilt runtime for `x86_64-linux-android` or `armv7-linux-androideabi`. |
| A threshold is appropriate | It is not, until measured against **this** model *and* **this** geometry on **this** input distribution. A threshold measured on centre-cropped web images does not describe tiled screen frames. Neither has a default that can be inherited. |
| A preprocessing path matches the reference | Pin it against the reference implementation with a fixture whose structure exposes position and offset (a channel ramp, not a flat colour). This caught a one-pixel `CenterCrop` rounding difference worth 0.0085 of tensor mean. |
| Two code paths score identically | Assert it on the real model, not on a synthetic buffer. |
| A bundled artifact is sealed | Append one byte to it and confirm `codesign --verify` now fails. |

## Browsers and third-party apps

| Claim shape | Falsifier |
|---|---|
| A browser trusts our CA | Render a real HTTPS page in that browser. **macOS `curl` is SecureTransport-built, ignores `--cacert`, and skips system proxy settings** — it cannot verify coverage for anything. |
| A browser policy is applied | Read the shipped logic. Firefox ignores `ImportEnterpriseRoots` when it is the only policy present, and `security.enterprise_roots.enabled` already defaults to `true`. |
| A browser uses the system resolver | Check the browser's own DNS setting. Chrome auto-upgrades to DoH where the resolver supports it; Android's Private DNS is system-wide DoT. Either makes a port-53 filter invisible. |

---

## Case log

Each row is a claim that was believed and built on, then measured false. Add to it.

| Package | Claim believed | Measured | Cost |
|---|---|---|---|
| `domain-blocklist` | `.com`/`.net`/`.org` support authenticated denial, so `Verdict::Dead` is reachable | NSEC3 opt-out; `AD` never set; `Dead` unreachable for almost every real domain | ~21 commits + 6 fix rounds |
| `domain-blocklist` | A nonexistent subdomain returns NXDOMAIN | Cloudflare zones return NODATA; the dead control's own worked example fails, aborting every sweep | same round |
| `image-sandbox` | Both v0 constants (size floor, threshold) were reasonable | Both wrong; floor remeasured to 96px, threshold has no shippable default at all | a redesign of the config surface |
| `mac-daemon` | `SCStreamConfiguration` delivers BGRA by default | Delivers biplanar `420v`; every frame was correctly refused; symptom indistinguishable from a missing Screen Recording grant | one session |
| `mac-daemon` | The point-vs-pixel fix was complete | `SCDisplay.width` is itself in points; stream ran at quarter resolution | same session |
| `mac-daemon` | `NSScreenCaptureUsageDescription` et al. exist | No usage-description key exists for any of the three capabilities; `CFBundleName` carries the whole message | contradicted the plan it was written from |
| `mac-daemon` | Chromium needs `AXManualAccessibility` | Chrome rejects it; Electron accepts and ignores it; what builds the tree is being an AX client at all | a module written around the wrong lever |
| `apps/mobile` | The VPN had the permissions it needed | `ACCESS_NETWORK_STATE` missing (threw after the TUN was up); `INTERNET` missing, and **that failure looks exactly like the filter working** | two emulator runs |
| `apps/mobile` (planned) | `ort` covers Android | No prebuilt runtime for two of three ABIs; forced the LiteRT/ONNX runtime split | a redesign |
