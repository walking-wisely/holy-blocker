# macOS Daemon — Implementation Plan

The per-platform capability analysis and the tamper-resistance model live in
[content-interception.md](../../decisions/content-interception.md) (§ "macOS — what carries over
and what changes"). The daemon responsibilities and scan cadence shared with the Windows daemon
are defined in [edge-daemons.md](../../architecture/edge-daemons.md). This document is the build
plan for `native-modules/mac-daemon/`: what modules to add, in what order, and what each one is
responsible for.

## Related flows

- [../flows/block.md](../../product/flows/block.md) — what happens when a scan returns Block
- [../flows/warn-interstitial.md](../../product/flows/warn-interstitial.md) — click-to-reveal cover on Warn verdict
- [../flows/protection-mode-change.md](../../product/flows/protection-mode-change.md) — how ProtectionMode propagates at runtime
- [../flows/partner-setup.md](../../product/flows/partner-setup.md) — the accountability-partner setup this platform's tamper model depends on

## Current state

`native-modules/mac-daemon/` is scaffolded as a SwiftPM package and Layer 1 is partially built:

- `Package.swift` — library target `MacDaemon`, executable `holy-blocker-macd`, test target
  `MacDaemonTests`. **Done.**
- `PrivilegedCommand.swift` — `CommandRunner` protocol, `SystemCommandRunner`, `SystemTool` paths.
  **Done.**
- `FakeCommandRunner.swift` — recording test double. **Done.**
- `NetworkServices.swift` — pure parsers for service lists, proxy settings, bypass domains.
  **Done.**
- `CATrust.swift` — System-keychain trust state, install, uninstall. **Done**, verified live on
  macOS 26.5: install → `installedAndTrusted`, confirmed independently by
  `security verify-cert` ("certificate verification successful"); second install raises no
  further admin prompt; uninstall leaves neither trust setting nor certificate.
- `ProxyConfiguration.swift` — snapshot / apply / restore, plus `DefaultBypass`. **Done**,
  verified live: a full apply → restore cycle across all four real network services returns the
  machine byte-identically to its prior state.
- `ProxySupervisor.swift` — `ProxySupervisorMachine` (the pure ordering state machine),
  `RestartBackoff`, and the executor that drives it through injected edges. **Done**, verified
  live: launch → health check → apply → SIGTERM → restore returns all four services
  byte-identically, and the child is reaped.
- `ProxyProcess.swift` — `MitmProxyProcess`, `TCPListenerProbe`, and the `SystemProxySettings`
  adapter over `ProxyConfiguration`. **Done.**
- `FirefoxTrust.swift` — `ImportEnterpriseRoots` policy in the `org.mozilla.firefox` preference
  domain, with a `CFPreferences`-backed store. **Written and unit-tested, but not working**: the
  policy lands correctly and `sudo firefox-trust` reports `enabled`, yet Firefox 152 still says the
  Enterprise Policies service is inactive. Firefox is therefore still uncovered — see the
  "unresolved" note in Layer 1 module 6.
- `holy-blocker-macd` — CLI verbs (`services`, `ca-status`/`ca-install`/`ca-uninstall`,
  `proxy-status`/`proxy-apply`/`proxy-restore`, `firefox-status`/`firefox-trust`/`firefox-untrust`,
  and `run`) so each module can be exercised against the real system. **Done.**

82 tests pass via `scripts/test.sh`.

**Layer 2 is now fully specified** (modules 0 and 7–14 below), superseding the sketch that stood
there. No Layer 2 code is written yet. Specifying it surfaced three things worth knowing before
implementation starts: the sketch was missing both a scan scheduler and an IPC module, the signed
bundle is a step-zero prerequisite rather than a later concern, and `PermissionGate` has to come
first rather than last. One product decision — what to do when the protected user is a local admin —
is called out in module 7 and needs sign-off before onboarding is written.

**Layer 1 is verified end to end, including a real browser.** With the supervisor running, a
`URLSession` fetch of `http://example.com` reached the proxy — `mitm_proxy::forward: forwarding
method=GET host=example.com port=80` — and returned 200. With the root CA installed and trusted via
`CATrust`, **Firefox 152 renders `https://example.com` through the proxy**, and `openssl s_client
-proxy 127.0.0.1:8080` reports `Verify return code: 0 (ok)` against our CA for a leaf issued
`CN=Holy Blocker Local CA` / `CN=example.com`. Layer 2 may begin.

Getting there required fixing a defect in `packages/mitm-proxy`: generated leaf certificates carried
`rcgen`'s default 1975→4096 validity and no extended key usage, so Firefox rejected the handshake
with `BadCertificate` regardless of root trust. See the mitm-proxy plan for the fix and, more
usefully, for why a chain-verification test would *not* have caught it.

**Firefox pins its own services.** Two background connections were still rejected with
`BadCertificate` after the fix, while `example.com` succeeded. These are almost certainly Firefox's
pinned Mozilla endpoints (telemetry / remote settings), which is the certificate-pinning limitation
already recorded under "What Layer 1 does not cover" — pinned clients fail closed rather than being
inspected. It could not be confirmed from the logs because **the proxy does not record the SNI when
a browser-side handshake fails**, which is a small observability gap worth closing before the next
coverage investigation.

### Two bugs the unit tests could not have caught

Both were found only by running against the real tools, and both are worth remembering before
writing fixtures for any further `security(1)` or `networksetup(8)` parsing:

1. **An invented fixture encoded a format that does not exist.** `state()` required a
   `Result = trustRoot` line in `dump-trust-settings` output. Real output has no such line: a
   trusted root is rendered with `Number of trust settings : 0`, an empty settings array meaning
   "trusted for all purposes" — all 157 built-in system roots appear exactly that way. The parser
   therefore reported `installedUntrusted` immediately after a successful install, making
   `install()` re-run `add-trusted-cert` on every daemon start and re-prompt for admin credentials
   each time. The unit test asserting idempotence passed against the fictional format.
2. **`restore()` left the proxy host behind.** `-setwebproxystate off` disables a proxy but keeps
   the server and port, so every service was left holding `127.0.0.1:8080`. Re-enabling the proxy
   for any unrelated reason would then route traffic at a dead local port. `-setwebproxy <service>
   "" 0` does blank both fields — undocumented but accepted — and must be issued *before* the
   state-off call, because it also turns the proxy on.

Capture fixtures from real tool output. Where that is impractical (the `deny` trust result is the
one remaining case), say so at the fixture.

### Do not verify proxy coverage with `curl`

macOS ships curl built against **SecureTransport** (`curl -V` reports it), and that build **ignores
`--cacert`** and does not consult the macOS system proxy settings. So `curl --cacert ca.crt
https://example.com` returns 200 whether or not the traffic was intercepted, and returns 200 again
with the flag removed. It looks like a passing end-to-end test and proves nothing in either
direction.

Use a CFNetwork client instead — a `URLSession` fetch honours the per-service proxy settings this
package writes, which is exactly the coverage question being asked. For the TLS half, `openssl
s_client -proxy 127.0.0.1:8080 -connect host:443 -servername host` prints the issuer actually
served, which is unambiguous. Both are recorded in the current-state section above.

What already exists and is reused unchanged:

- `packages/mitm-proxy/` — Rust. Plain HTTP forwarding, TLS state and per-SNI cert generation,
  CONNECT handler, and the HTTP/1.1 tunnel loop with scan hooks. Portable; runs on macOS today.
- `packages/text-policy/` — Rust. The classification engine the proxy already calls.

Unlike iOS, macOS is a **full-tier platform**: both interception layers run and our own
classification engines participate. The only capability lost relative to Windows is the deferred
process-injection optimization, which is not in the core model.

## Scope split — what belongs in this package

This package is the **macOS platform adapter**. Policy and classification stay in the Rust
packages; the daemon owns Apple APIs, permission handling, and process lifecycle.

| Concern | Where it lives |
|---|---|
| Verdicts, scoring, lexicon | `packages/text-policy` (Rust) — unchanged |
| TLS termination, URL/body scanning | `packages/mitm-proxy` (Rust) — unchanged |
| CA trust, system proxy config, process supervision | **this package**, Layer 1 |
| Screen capture, overlay, AX/event hooks | **this package**, Layer 2 |

Language is **Swift**, built with SwiftPM. Keep Apple API glue thin and put decision logic in
pure, testable types — the same rule the Windows daemon follows for Win32 glue.

## Build environment constraint

Layer 1 was built on **Swift 6.3 with Command Line Tools only — no full Xcode**, which was
sufficient for all of it (a plain SwiftPM executable, no app bundle, no entitlements).

**That constraint has since lifted.** As of macOS 26.5.2, `xcode-select -p` on the development
machine reports `/Applications/Xcode.app` (Swift 6.3.3), and `ScreenCaptureKit`,
`ApplicationServices`, `IOKit.hid` and `AppKit` were all confirmed to import, link and run against
this toolchain. The `scripts/test.sh` workaround below is now a no-op here, but keep it — it tests
the active toolchain, not the presence of Xcode, and Layer 1 must stay buildable under Command Line
Tools.

Layer 2 still needs a signed `.app` bundle for stable TCC permission grants, since TCC identifies
clients by code signature and unsigned command-line binaries get inconsistent, easily-invalidated
grants. **That bundle is the *first* Layer 2 step, not a later one** — see Layer 2 module 0. An
earlier draft of this paragraph deferred it to "step 5", which was wrong: every permission grant
made before the bundle exists is invalidated by the next rebuild, so building capture first means
granting Screen Recording to an identity that is about to change.

### Running the tests — use `scripts/test.sh`, not `swift test`

Two toolchain quirks, both already worked around, both worth knowing before touching the manifest:

1. **`swift test` alone cannot find swift-testing under Command Line Tools.** `Testing.framework`
   ships at `/Library/Developer/CommandLineTools/Library/Developer/Frameworks`, which SwiftPM does
   not search, so the test target fails to compile `import Testing`; adding only `-F` gets it
   compiling but it then fails to `dlopen` both `Testing.framework` and `lib_TestingInterop.dylib`.
   `scripts/test.sh` supplies the search path and both rpaths, and is a no-op when the active
   toolchain is a full Xcode.
2. **Those flags must not move into `Package.swift`.** A test target carrying `unsafeFlags` makes
   SwiftPM **silently stop discovering tests** — `swift test` builds, prints nothing, exits 0, and
   `swift test list` reports nothing. There is no warning. A green-looking run that executed zero
   tests is the worst possible failure mode for a test-first package, so the flags stay in the
   script.

Note also that `xcode-select -p` may point at the Command Line Tools even when `Xcode.app` is
installed; the script tests the active toolchain, not the presence of Xcode.

---

# Layer 1 — network path

The proxy itself is done. What is missing is everything that makes macOS traffic actually arrive at
it, and everything that makes the daemon a well-behaved system citizen.

## Modules to add

### 1. `PrivilegedCommand` — the process-execution edge

```
Sources/MacDaemon/PrivilegedCommand.swift
```

Both Layer 1 modules work by invoking Apple command-line tools. To keep them testable without
mutating the real keychain or network configuration, execution sits behind one protocol:

```swift
struct CommandResult {
    let exitCode: Int32
    let standardOutput: String
    let standardError: String
}

protocol CommandRunner {
    func run(_ executable: String, _ arguments: [String]) throws -> CommandResult
}
```

Responsibilities:

- `SystemCommandRunner` — the real implementation, wrapping `Foundation.Process`. Uses absolute
  executable paths (never `PATH` lookup) so behaviour cannot be hijacked by the protected user's
  environment.
- `FakeCommandRunner` — test double. Records the exact argument vectors it was handed and returns
  scripted `CommandResult` values.

Every module below is constructed with a `CommandRunner` and never touches `Process` directly. This
is what makes "did we build the right argv?" and "did we parse the output correctly?" unit-testable
with no side effects, which is the bulk of the logic in this layer.

### 2. `CATrust` — root CA installation in the System keychain

```
Sources/MacDaemon/CATrust.swift
```

Responsibilities:

- Determine whether the Holy Blocker root CA is already present and trusted.
- Install it into the **System** keychain (`/Library/Keychains/System.keychain`) as a trusted root,
  which is what makes the proxy's per-SNI leaf certificates validate in Safari, Chrome, and every
  CFNetwork client.
- Remove it again on uninstall, leaving no trust residue.

The install is **admin-gated by the OS** — writing the System keychain requires administrator
authorization. This is a feature, not an obstacle: it is the same admin credential the partner
holds in the tamper model, so the protected user cannot quietly remove the CA.

Key signatures:

```swift
enum CATrustState {
    case absent
    case installedAndTrusted
    case installedUntrusted   // present but trust settings missing or denied
}

struct CATrust {
    init(runner: CommandRunner, certificatePath: URL)

    func state() throws -> CATrustState
    func install() throws
    func uninstall() throws
}
```

Pure logic to test first, with a `FakeCommandRunner`:

- Argument-vector construction for install, verify, and remove — including the keychain path, the
  `-d` (admin cert store) and `-r trustRoot` flags, and correct quoting of a certificate path
  containing spaces.
- Parsing tool output into `CATrustState`, including the "not found" exit status, which is a normal
  first-run condition and must not be treated as an error.
- Idempotence: calling `install()` when already in `installedAndTrusted` must not shell out again.

Note that Firefox maintains **its own NSS trust store** and does not consult the System keychain.
Firefox coverage therefore needs a separate step (its `security.enterprise_roots.enabled`
preference, or direct NSS import). This is deliberately **out of scope for step 2** — record it as a
known gap and handle it in Layer 1 step 6.

#### Reference documents

- [`security(1)` man page](https://keith.github.io/xcode-man-pages/security.1.html) — the
  authoritative reference for `add-trusted-cert`, `remove-trusted-cert`, and `find-certificate`,
  including the `-d`, `-r`, and `-k` flag semantics used here.
- [Apple — Certificate, Key, and Trust Services](https://developer.apple.com/documentation/security/certificate_key_and_trust_services)
  — the framework-level model behind what `security` manipulates; needed if the shell-out is ever
  replaced with direct `SecTrustSettings` calls.
- [Apple — Requirements for trusted certificates in iOS 13 and macOS 10.15](https://support.apple.com/en-us/103769)
  — constrains what leaf certificates the proxy may generate (maximum validity period, required
  EKU, SAN requirements). The proxy's `rcgen` configuration must satisfy these or Safari rejects
  the leaf even with the root trusted.

### 3. `NetworkServices` — enumerating and parsing network services

```
Sources/MacDaemon/NetworkServices.swift
```

macOS proxy settings are **per network service** (Wi-Fi, Ethernet, Thunderbolt Bridge, each VPN),
not global. Configuring the proxy means iterating every service. This module is the pure parsing
half of that job.

Responsibilities:

- Parse the output of `networksetup -listallnetworkservices` into structured values.
- Handle the two parsing hazards explicitly: the output begins with a **header line** ("An asterisk
  (*) denotes that a network service is disabled."), and **disabled services are prefixed with
  `*`**. Passing a `*`-prefixed name back to `networksetup` fails, so the prefix must be stripped
  and the disabled state retained separately.
- Parse `networksetup -getwebproxy <service>` output into current proxy state, so prior settings can
  be restored later.

Key types:

```swift
struct NetworkService: Equatable {
    let name: String
    let isEnabled: Bool
}

struct ProxySetting: Equatable {
    let isEnabled: Bool
    let server: String
    let port: Int
}

enum NetworkServices {
    static func parseServiceList(_ output: String) -> [NetworkService]
    static func parseProxySetting(_ output: String) -> ProxySetting?
}
```

Both parsers are **pure string functions** with no `CommandRunner` dependency — the easiest and
highest-value tests in this layer. Test against captured real output including: the header line,
disabled `*`-prefixed entries, service names containing spaces, an empty list, and malformed input.

### 4. `ProxyConfiguration` — pointing macOS at the proxy, and putting it back

```
Sources/MacDaemon/ProxyConfiguration.swift
```

Responsibilities:

- For every enabled network service, set the web proxy and secure web proxy to the local
  `mitm-proxy` listener, and apply the bypass list.
- **Snapshot the prior state of every service before modifying it**, and restore that exact state on
  stop or uninstall. The daemon must leave the machine as it found it — including services that
  already had a proxy configured for unrelated reasons.
- Persist the snapshot to disk, because the daemon can be killed between configure and restore. A
  crash must not strand the user behind a dead proxy with no route back.

Key signatures:

```swift
struct ProxySnapshot: Codable, Equatable {
    let service: String
    let web: ProxySetting?
    let secureWeb: ProxySetting?
    let bypassDomains: [String]
}

struct ProxyConfiguration {
    init(runner: CommandRunner, snapshotPath: URL)

    func snapshot(services: [NetworkService]) throws -> [ProxySnapshot]
    func apply(host: String, port: Int, bypass: [String], to services: [NetworkService]) throws
    func restore() throws
}
```

Test first, with `FakeCommandRunner`:

- Argv construction for `-setwebproxy`, `-setsecurewebproxy`, `-setwebproxystate off`, and
  `-setproxybypassdomains`, including a service name with spaces passed as a single argument.
- Round-trip: `snapshot` → `apply` → `restore` issues commands that return each service to its
  captured state, with a service that had no prior proxy correctly ending up **off** rather than
  pointed at a stale server.
- Disabled services are skipped.
- `restore()` with a snapshot file left by a previous crashed run is honoured on next startup.

**`restore()` is not atomic per service.** Clearing a service takes two calls —
`-setwebproxy <svc> "" 0` to blank the host, then `-setwebproxystate <svc> off` — and the order
cannot be reversed, because blanking also switches the proxy *on*. A process death between the two
therefore leaves the service **enabled with an empty server**, which is worse than either endpoint
and can break browsing outright. Observed once during development, when the supervisor was killed
mid-restore.

The persisted snapshot is what makes this recoverable: it is not deleted until the restore
completes, so `proxy-restore` on the next start replays it and lands correctly. That is the crash
path working as designed, but a startup check that detects and repairs "enabled with an empty
server" would close the window properly, and is not yet written.

**Bypass list.** At minimum `localhost`, `127.0.0.1`, `*.local`, and the link-local range, so the
proxy never sits in front of loopback and Bonjour traffic. Captive-portal detection hosts belong
here too or the user cannot join a hotel network while protected.

#### Reference documents

- [`networksetup(8)` man page](https://keith.github.io/xcode-man-pages/networksetup.8.html) — the
  authoritative reference for every subcommand used here. Note the argument order for
  `-setwebproxy <service> <domain> <port> <authenticated> <username> <password>`; the trailing
  authentication arguments are optional but positional.
- [Apple — `SystemConfiguration` framework](https://developer.apple.com/documentation/systemconfiguration)
  — the API `networksetup` is a front-end for. Relevant if shelling out proves too slow or fragile
  and the daemon moves to `SCPreferences` directly; also documents the
  `kSCPropNetProxies*` keys that appear in snapshots.
- [Apple — `CFNetwork` proxy support](https://developer.apple.com/documentation/cfnetwork/cfproxysupport)
  — defines *which* clients honour these system settings, which is the coverage question below.

### 5. `ProxySupervisor` — running and monitoring the Rust proxy

```
Sources/MacDaemon/ProxySupervisor.swift
```

Responsibilities:

- Launch the `mitm-proxy` binary as a child process bound to `127.0.0.1` on a chosen port.
- Health-check the listener before configuring system proxy settings — configuring first and
  launching second would black-hole all traffic during the gap.
- Restart on unexpected exit with backoff; if the proxy cannot be kept alive, **restore proxy
  settings** rather than leaving the machine pointed at a dead listener. Fail open on the network
  path, never fail closed into a broken network.
- Ordering guarantee, which is the part worth testing: configure only after healthy, and restore
  before terminating.

Model the ordering as a pure state machine (`stopped → starting → healthy → configured →
restoring`) so the transitions are unit-testable without spawning anything.

**A third ordering rule emerged while building this, and it is the least obvious of the three:
system proxy settings must be applied exactly once per run.** `ProxyConfiguration.apply()`
snapshots current settings before writing its own, so re-applying after a restart would capture
*our* proxy as the machine's prior state — and `restore()` would then pin the user to a dead local
port permanently, with the real prior settings gone. The machine therefore tracks whether this run
already owns a snapshot, and a restart that comes back healthy transitions straight to `configured`
without re-applying.

**`mitm-proxy` currently takes no command-line arguments.** It hardcodes `127.0.0.1:8080` and
loads its CA from the relative path `data/ca`, so the child's working directory is load-bearing and
the port is not actually selectable yet. `MitmProxyProcess` accepts an `arguments` array so this
side needs no change once the Rust binary grows a CLI, but until it does, `HOLY_BLOCKER_PROXY_PORT`
only changes where the supervisor *probes*, not where the proxy *binds*. Giving `mitm-proxy` a
`--port` and `--ca-dir` is a prerequisite for running on any other port.

### 6. Firefox NSS trust — `FirefoxTrust.swift`

Firefox keeps its own NSS trust store and ignores the System keychain, so `CATrust` alone leaves
every Firefox user staring at certificate errors. The `Certificates` → `ImportEnterpriseRoots`
enterprise policy sets `security.enterprise_roots.enabled`, which makes Firefox read the platform
store. It is supported on Windows and macOS only.

An earlier draft of this section named
`/Library/Application Support/Mozilla/Certificates / policies.json` as the policy location. **No
such path exists.** Mozilla documents exactly two delivery mechanisms on macOS:

1. **The managed-preferences domain `org.mozilla.firefox`** — what this module uses. Policy keys
   sit at the top level of the domain: `EnterprisePoliciesEnabled` as a boolean, and `Certificates`
   as a nested dictionary containing `ImportEnterpriseRoots`.
2. **`Firefox.app/Contents/Resources/distribution/policies.json`** — rejected. Writing inside the
   bundle breaks the notarized app's code-signature seal, and every Firefox update replaces the
   bundle and silently drops the policy.

Reads and writes go through `CFPreferences`, not the plist file, because that is the API Firefox
itself reads with; writing the file directly races `cfprefsd`, which caches the domain and can
serve or write back a stale copy. `kCFPreferencesAnyUser` + `kCFPreferencesAnyHost` is the pair
that maps to `/Library/Preferences/org.mozilla.firefox.plist` — `kCFPreferencesCurrentHost` would
land in `/Library/Preferences/ByHost/` under a hardware UUID instead. Writing requires root, which
suits the tamper model: the protected user cannot revoke it unprivileged.

The merge behaviour is the part worth testing, and it mirrors the third-party-proxy concern
recorded below. An MDM-managed Mac may already carry a Firefox policy payload, so `install()`
merges into whatever is there and `uninstall()` removes only `ImportEnterpriseRoots` — dropping the
`Certificates` dictionary only if it empties, and `EnterprisePoliciesEnabled` only if no other
policy still depends on it.

Direct NSS import into each profile with `certutil` remains the fallback for versions that ignore
the policy. It is fragile — it requires locating every profile and a tool Firefox does not install
— and is not built.

#### This module is not needed for coverage, and cannot work the way it is written

Two findings, both from reading the shipped Firefox 152.0.5 build rather than reasoning about it.
Together they retire the premise this step was written on.

**1. Firefox already trusts System-keychain roots.** `security.enterprise_roots.enabled` is a
`StaticPref` with a default of `true`. There is nothing to turn on. The original claim at the top of
module 2 — that Firefox ignores the System keychain and therefore needs a separate step — has been
obsolete for several releases.

**2. Firefox deliberately ignores this exact policy when it stands alone.** From
`modules/EnterprisePoliciesParent.sys.mjs` inside `Firefox.app/Contents/Resources/omni.ja`:

```js
// Because security.enterprise_roots.enabled is true by default, we can
// ignore attempts by Antivirus to try to set it via policy.
if (
  Object.keys(provider.policies).length === 1 &&
  provider.policies.Certificates &&
  Object.keys(provider.policies.Certificates).length === 1 &&
  (provider.policies.Certificates.ImportEnterpriseRoots === true ||
    provider.policies.Certificates.ImportEnterpriseRoots === 1)
) {
  this.status = Ci.nsIEnterprisePolicies.INACTIVE;
  return;
}
```

A policy set consisting of *only* `Certificates.ImportEnterpriseRoots` short-circuits to INACTIVE
and returns before `_activatePolicies` — so the policy is genuinely not applied, not merely
reported oddly. **To make `ImportEnterpriseRoots` take effect via policy you must ship at least one
other policy alongside it.** There is no warning; `about:policies` just says the service is
inactive.

Note what this does *not* mean. `forced`-ness was a dead end: hand-creating
`/Library/Managed Preferences/org.mozilla.firefox.plist` does flip
`CFPreferencesAppValueIsForced` to true (measured), and Firefox still reported inactive, because the
short-circuit above is the real gate. Mozilla's KB advertising
`sudo defaults write /Library/Preferences/org.mozilla.firefox ...` is fine as far as it goes.

##### What the module is actually for

Keep it, but on a different justification. It is useless as "make Firefox trust the CA" and
potentially useful as **"re-assert the pref if something turns it off"** — a user or an antivirus
setting `security.enterprise_roots.enabled` to false is a plausible evasion of the whole render-path
model. That is a tamper-model concern, not a coverage one, and any implementation of it has to
carry a companion policy or hit the short-circuit above.

##### Verifying this without a human

`about:policies` is a GUI page, but Firefox renders it headlessly, which makes this checkable from
CI or an agent:

```
/Applications/Firefox.app/Contents/MacOS/firefox --headless --new-instance \
  -profile <tmp-profile> --window-size 1200,900 \
  --screenshot <out.png> about:policies
```

Use a throwaway `-profile` so a running Firefox is undisturbed. The shipped policy logic itself is
readable without launching anything — `omni.ja` is a plain zip:

```
unzip -p /Applications/Firefox.app/Contents/Resources/omni.ja \
  modules/EnterprisePoliciesParent.sys.mjs
```

Prefer that to reading `mozilla-central`, which is several versions ahead of any installed build and
had already refactored this code path.

#### Reference documents

- [Firefox enterprise policy reference — Certificates](https://firefox-admin-docs.mozilla.org/reference/policies/certificates/)
  — `ImportEnterpriseRoots` semantics, platform support, and the preference it maps to.
- [Mozilla policy templates — `mac/org.mozilla.firefox.plist`](https://github.com/mozilla/policy-templates/blob/master/mac/org.mozilla.firefox.plist)
  — the authoritative plist shape; confirms the keys are top-level with no wrapping container.
- [Apple — `CFPreferences`](https://developer.apple.com/documentation/corefoundation/preferences_utilities)
  — the user/host domain pairs and which file each resolves to.

## What Layer 1 does *not* cover on macOS — honest limits

These are coverage gaps inherent to the system-proxy approach and must be recorded, not discovered
later:

- **QUIC / HTTP3 over UDP 443** bypasses an HTTP proxy entirely. Chrome disables QUIC when a system
  proxy is configured, but this is browser policy, not a guarantee. A complete answer needs UDP 443
  blocking, which belongs to the transparent-proxy path below, not here.
- **Apps that ignore system proxy settings** — anything using raw `BSD sockets`, hardcoded
  networking, or its own proxy configuration. CFNetwork/`URLSession` clients and the major browsers
  do honour them; command-line tools generally do not.
- **Certificate pinning** — pinned apps will fail to connect rather than be inspected. The bypass
  list is the mitigation.
- **The protected user can change proxy settings back** unless they are a standard user. This is
  exactly why the tamper model is an account-model question, not a code question — and it is
  sharper than it first appears: **`networksetup` proxy changes require no root at all for an
  admin user** (verified on macOS 26.5; `-setwebproxy` succeeds and mutates from an unprivileged
  shell). Only the CA install and the default state directory need privilege. So on a machine
  where the protected user is a local admin, Layer 1 is trivially reversible by them, and the
  standard-user split is not a hardening option but a prerequisite.

### Snapshot/restore assumes exclusive ownership of the proxy setting

`ProxyConfiguration` reads the prior state once, then writes it back on stop. That is only correct
if nothing else touches proxy settings in between — and on a machine running this software, that
assumption is weaker than usual, because the people who install a content blocker frequently
already run another one.

Three failure modes, none currently handled:

1. **Another tool sets a proxy while ours is applied.** Our `restore()` writes back the value
   captured at start, silently reverting their change.
2. **Another tool owned the proxy when we started.** `apply()` overrides it, disabling *their*
   filtering for as long as we run. For a blocker, quietly turning off a competing blocker is the
   worst possible bug — it is a bypass.
3. **Our snapshot is stale after a crash.** The on-disk snapshot is replayed on next start, which
   may now be older than the user's own settings.

Mitigation when this is built: re-read the current value immediately before restoring and only
write back if it still matches what we applied (compare-and-swap rather than blind restore), and
refuse to `apply()` over a pre-existing third-party proxy without explicit user consent.

Verified on the development machine (macOS 26.5): **Cold Turkey Blocker** is installed and running
there but does *not* use this surface — it blocks through browser extensions via a native
messaging host, leaves `/etc/hosts` untouched, and installs no root CA. So it composes with
Layer 1 rather than conflicting. Tools that *do* take the system proxy (Charles, Proxyman, mitm
tooling, some VPN clients, corporate MDM proxy profiles) will conflict on a last-writer-wins basis.

**A second peer product is present and was missed by that survey.** `systemextensionsctl list` on
the same machine shows `com.Cvnt.ce.FirewallExtension` — **Covenant Eyes** — alongside Tailscale's
network extension. It is a `NetworkExtension` content filter, not a system-proxy consumer, so it
also composes with Layer 1; but it occupies the surface the transparent-proxy path below would want,
and two filters on that surface is a genuine conflict to plan for rather than discover. Two things
follow beyond the conflict question: the leading product in precisely this category chose the
**network** boundary over the exec boundary, which is a strong signal about what is practically
approvable; and it runs there with the user as a local admin, which is the configuration the tamper
model treats as weakened.

### The transparent-proxy alternative, and why it is deferred

`NETransparentProxyProvider` / `NEAppProxyProvider` (a Network Extension system extension) would
capture traffic regardless of app proxy support and is the true macOS analogue of net-shield's
Wintun path. It is deferred because it requires a **gated Apple entitlement**, a signed and
notarized app bundle, and full Xcode — weeks of lead time — whereas the `networksetup` path works
today with zero entitlements and covers the browsers that matter most.

There is a second, strategic reason to build it eventually: `NEFilterDataProvider` on macOS is the
**same API as the iOS content filter**, so the macOS extension is the development and debugging
environment for the iOS build. See the iOS section of
[content-interception.md](../../decisions/content-interception.md).

Note that its entitlement is the **more routinely granted** of the two gated options — it is what
Covenant Eyes ships — so if an entitlement request is going to be made at all, this is the one with
the shorter path and the iOS payoff.

### Endpoint Security, and why it is deferred rather than rejected

`ES_EVENT_TYPE_AUTH_EXEC` is the only sound place to authorize command execution: the client is
called before `execve` completes with the resolved path, full argv and code-signing identity. It sits
below the shell, so quoting, `base64`, variable assembly and `eval` are all already resolved, and it
covers binaries — which no text-inspection scheme can. Confirmed present on macOS 26.5.2 as
`libEndpointSecurity.tbd` with headers under `$SDK/usr/include/EndpointSecurity/`. It also supplies
the self-defence events Android gets from DeviceAdmin: `AUTH_SIGNAL` (deny `SIGKILL`),
`AUTH_UNLINK` / `AUTH_RENAME` (deny removal), `AUTH_GET_TASK` (deny debugger attach).

**Two constraints to record before anyone starts.** First, the entitlement
`com.apple.developer.endpoint-security.client` is Apple-gated and needs a notarized System Extension.
Second, and less obvious: AUTH messages carry a per-message `deadline`, and the header states that a
client failing to answer before it **will be killed** — so a decision cache keyed on the binary hash
(`es_respond_auth_result`'s cache flag, `es_clear_cache`) is mandatory rather than an optimisation.

It is **deferred, not rejected**, because its marginal value over root `LaunchDaemon` + standard user
is narrow: against a standard user those already block the routes ES would block, and against an
admin the extension is itself removable. See
[content-interception.md](../../decisions/content-interception.md) for the full argument, including
why `systemextensionsctl developer on` cannot ship (it needs SIP off, and SIP off makes `TCC.db`
directly writable — a larger hole than the one being closed) and why a kernel extension is strictly
dominated by this path.

## Layer 1 implementation order

1. ~~Scaffold the SwiftPM package: `Package.swift` with one executable target and one test target.
   Confirm `swift build` and `swift test` both run under Command Line Tools with no Xcode.~~
   **Done.** Test invocation is `scripts/test.sh` — see the build-environment section above.
2. ~~`PrivilegedCommand.swift` — the `CommandRunner` protocol, the real runner, and the fake.
   Nothing else can be tested until this exists.~~ **Done.**
3. ~~`NetworkServices.swift` — pure parsers. Tests first; these need no fake runner at all.~~
   **Done.** Fixtures are verbatim `networksetup` output captured on macOS 26.5.
4. ~~`CATrust.swift` — argv construction and state parsing under test, then the real install path,
   verified once by hand against the System keychain.~~ **Done.** All fixtures confirmed against
   the real `security(1)`; install/idempotence/uninstall verified live, with `verify-cert` as an
   independent check that the CA was actually trusted.
5. ~~`ProxyConfiguration.swift` — snapshot/apply/restore under test, then a manual end to end
   check that `restore()` genuinely returns the machine to its prior state.~~ **Done** —
   apply → restore verified byte-identical across all four real network services.~~ Traffic
   through the proxy is now confirmed too — see the end-to-end note in the current-state section.
6. ~~`ProxySupervisor.swift` — the ordering state machine under test, then real process
   spawning.~~ **Done.** The state machine, `RestartBackoff`, and the executor are unit-tested;
   the `run` verb was exercised live against a real `mitm-proxy` child, including SIGTERM →
   restore → reap.
7. ~~Firefox NSS trust (module 6 above).~~ **Retired as a coverage step.** Firefox trusts
   System-keychain roots by default (`security.enterprise_roots.enabled` defaults to `true`), so
   `CATrust` alone covers it and no policy is required. `FirefoxTrust.swift` is kept for the
   tamper-model case only; read module 6 before touching it, because Firefox silently ignores
   `ImportEnterpriseRoots` when it is the only policy present.

---

# Layer 2 — render path

Conceptually identical to the Windows daemon — capture → text + image models → mode-driven response
— but every mechanism is an Apple API behind a TCC permission, and that changes the shape of the
work: on Windows the hard part is the capture, here the hard part is *holding* a permission the OS
deliberately keeps under user control.

**Layer 1 has landed**, so the sketch that stood here is now replaced by a full specification.

## What changed between the sketch and this specification

Four things, recorded because they alter the plan rather than merely detail it:

1. **Full Xcode is now installed** — `xcode-select -p` reports `/Applications/Xcode.app` on the
   development machine (macOS 26.5.2, Swift 6.3.3). The build-environment section above was written
   under Command Line Tools only. `ScreenCaptureKit`, `ApplicationServices`, `IOKit.hid` and
   `AppKit` all import and link, verified by compiling and running a probe against this toolchain.
   The bundle-signing prerequisite Layer 2 needs is therefore no longer blocked on a toolchain
   install.
2. **The sketch was missing two modules.** It listed capture, overlay, hooks, AX text, fullscreen
   and permissions — but nothing that *schedules* scans (the Windows daemon's `scan_loop`) and
   nothing that carries verdicts to the Electron app (the Windows daemon's `ipc`). Both are load
   bearing and both are specified below. Without the scheduler there is no debounce, no cadence
   split between the image and text models, and nowhere for `ProtectionMode` to be applied; without
   the IPC module the daemon can detect but cannot respond.
3. **`PermissionGate` moves from last to first.** It was numbered 12 because it reads like
   onboarding polish. It is not: no other Layer 2 module can be *run*, let alone verified, until
   the process holds the permission it needs, and the permission is attached to a code signature
   that must exist before the first grant. Building capture first means granting Screen Recording
   to a binary whose identity is about to change.
4. **Permission loss is a tamper event, not an error path.** This is the lesson the Android work
   already paid for — see the `GuardStatus` foreground-service note in the root `AGENTS.md`: the
   thing that matters is the still-alive process that can record a disable the guard itself cannot
   report. Screen Recording is revocable by anyone who can authenticate as an admin, and Apple
   reserves that right permanently (no MDM can remove it). So the daemon must treat "I no longer
   have capture" as a reportable state transition, not as a reason to log and exit.

## Module 0 — the signed bundle, and the TCC identity trap

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

### Built — and one instruction above is wrong

**Done**, including a stable signing identity — see the end of this section and
[signing-identity.md](signing-identity.md). Shipped: `AppBundle` (layout, `Info.plist` generation, `assemble`),
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
[signing-identity.md](signing-identity.md) for the runbook — where the identity lives, how to
recreate it if lost, and how to rotate it. This is still a development-only identity; a Developer
ID certificate is a separate, later decision needed before shipping to anyone else's machine (see
that page's "Before shipping to anyone else"). Backlog item 1's remaining blocker is now only the
human step: granting Screen Recording / Accessibility to the signed bundle and observing a real
revocation.

---

## Modules to add

### 7. `PermissionGate` — TCC state, and the account model that locks it

```
Sources/MacDaemon/PermissionGate.swift
```

**Built, except for the live grant/revocation check — see the notes at the end of this section.**

Detect and report which permissions are held, and detect the configuration that silently defeats
the tamper model.

```swift
enum PermissionState { case granted, denied, undetermined }

enum Capability { case screenRecording, accessibility, inputMonitoring }

struct PermissionSnapshot {
    var screenRecording: PermissionState
    var accessibility:   PermissionState
    var inputMonitoring: PermissionState

    // Environment signals — none of these needs a permission. See "the tamper surface" below.
    var protectedUserIsAdmin: Bool
    var sipEnabled:           Bool
    var localUsers:           [String]
    var adminGroupMembers:    [String]
    var bootTime:             Date
}
```

Responsibilities:

- Read state through the **preflight APIs**, which do not prompt:
  `CGPreflightScreenCaptureAccess()`, `AXIsProcessTrusted()`, and
  `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)`. All three were verified to compile and run
  against the current toolchain; on an ungranted machine they return `false`, `false`, and
  `kIOHIDAccessTypeUnknown` (raw value 2) respectively.
- **Do not read `TCC.db`.** It is SIP-protected, reading it requires Full Disk Access — a *stronger*
  permission than the ones being inspected, which is an absurd dependency — and its schema is
  private and has changed across releases. The preflight APIs are the supported surface.
- Prompt exactly once per capability, deliberately, from the onboarding path — never as a side
  effect of a scan. `CGRequestScreenCaptureAccess()` and
  `AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt: true])`. Note that Screen Recording's
  prompt cannot be re-shown once denied; the only recovery is deep-linking the user to
  System Settings (`x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`).
- **Poll for revocation** and emit a state transition when a held permission is lost. This is the
  Android `GuardStatus` pattern, and it is the module's most important job — a revoked Screen
  Recording grant is indistinguishable from "nothing bad is on screen" unless it is detected
  explicitly.
- Detect whether the protected user is a local administrator, which is the open product question
  below.

The pure part — mapping a `PermissionSnapshot` to a "what is actually protected right now"
assessment and to state transitions worth reporting — is a plain function over the struct and gets
tests first. The three preflight calls are the edge and go behind a protocol, mirroring
`CommandRunner`.

#### This module is the tamper surface — watch outcomes, not routes

The design goal under the admin model is **"no bypass route that is both easy and silent"** (see
[content-interception.md](../../decisions/content-interception.md) for the route-by-route coverage
table). `PermissionGate` is where nearly all of that lands, and the reason is a single observation:
**revoking Screen Recording through System Settings and running `tccutil reset` produce the same
outcome, so one poll detects both.** Guarding a route covers one door; watching the outcome covers
every door that leads to it.

Four extra environment signals belong in the snapshot. All four were verified readable on macOS
26.5.2 with **no permissions granted at all** — no Screen Recording, no Accessibility:

| Signal | Source | Detects |
|---|---|---|
| SIP state | `csrutil status` | the precondition for every deep bypass (SIP-off makes `TCC.db` directly writable) |
| Local users | `dscl . -list /Users` | a second account created to escape the guard |
| Admin members | `dscl . -read /Groups/admin GroupMembership` | the protected user self-granting admin |
| Boot time | `sysctl kern.boottime` | correlating an unclean stop with a reboot |

The last one is the Android lesson transplanted: `TamperLog.classifyConnect` anchors on session
boundaries rather than reading a single line, and the same reasoning applies here — a daemon that
died and a machine that rebooted are different events and must not be conflated.

Transitions from this module feed a macOS tamper log modelled directly on `apps/mobile`'s
`policy/TamperLog.kt` (pure) + `TamperLogStore.kt` (the filesystem edge) split. That is a port of a
proven design, not a new one.

#### Admin detection, and the product decision it forces

The tamper model in
[content-interception.md](../../decisions/content-interception.md) requires the protected person to
be a **standard user** with an accountability partner holding the admin password. If the protected
user is themselves an admin, every Layer 2 permission is revocable by them in two clicks, and —
as recorded in Layer 1's limits — `networksetup` proxy changes need no root for an admin either, so
Layer 1 falls with it. The entire tamper model is then decorative.

Detection is a group-membership read: `dscl . -read /Groups/admin GroupMembership`, or `id -Gn` for
the current user. **On the development machine this check currently returns true** — `admin` appears
in `id -Gn`, and `GroupMembership: root dutov _mbsetupuser` — so the machine Layer 2 is being built
on is itself the unprotected configuration. That is worth knowing before any tamper-resistance claim
is made from local testing.

The product decision is what to *do* about it, and it is genuinely open. Three options:

| Option | Consequence |
|---|---|
| Refuse to run | Honest, and useless for most Mac users — the majority are their own sole admin. Guarantees the product is uninstalled during onboarding. |
| Warn, run anyway, and **report the weakened state** | Onboarding names the limitation plainly; the daemon records it and, in partnered mode, surfaces it to the partner. |
| Proceed silently | Rejected outright. The product would be claiming protection it does not have. |

**Recommendation: the middle option.** It matches the accountability model in
[accountability.md](../../decisions/accountability.md), where solo mode is a complete configuration
rather than a failure state — a self-admin user is in the same category, protected by friction and
their own intent rather than by a lock, and the tool should say so rather than either pretending
otherwise or refusing to help. The one thing that must not happen is the status UI showing the same
"protected" state in both configurations. This needs sign-off before onboarding is written; it is
listed as an open question in
[content-interception.md](../../decisions/content-interception.md) and should be resolved there,
not here.

#### What was built, and five things that only showed up in the writing

`PermissionGate.swift` ships `PermissionState` / `Capability` / `PermissionSnapshot` as specified,
plus `ProtectionAssessment` (the "what is actually protected" mapping), `TamperEvent` (the
transitions), `SystemEnvironment` (pure parsers for the four zero-permission signals),
`PermissionProbing` + `SystemPermissionProbe` + `FakePermissionProbe` (the preflight edge, mirroring
`CommandRunner`), and a `permissions` CLI verb. 38 tests.

1. **`CGPreflightScreenCaptureAccess` cannot distinguish denied from never-asked** — it returns a
   `Bool`, so two of the three `PermissionState` cases collapse. Not-granted is reported as
   `undetermined`, because the two states have different recovery paths and `undetermined` is the
   recoverable one; guessing `denied` would send onboarding to System Settings when a prompt would
   still work. Only `IOHIDCheckAccess` reports all three states.
2. **`csrutil status` has a third answer.** With individual protections toggled it prints
   `unknown (Custom Configuration)`, not `disabled`. That is parsed as *not* enabled — it is
   precisely the partially-disabled state the signal exists to catch — while genuinely
   unrecognised output returns `nil` and makes `snapshot()` throw rather than default.
3. **`dscl . -list /Users` alone is unusable** — it returns 133 rows on the development machine,
   all but one of them service accounts, so a bypass account created to escape the guard would be
   invisible in the noise. The listing is taken with `UniqueID` and filtered to UID ≥ 500 (Apple
   reserves everything below for the system; the first human account is 501), which reduces it to
   the one real name.
4. **`kAXTrustedCheckOptionPrompt` does not compile under Swift 6.** It is imported as a mutable
   global (`extern CFStringRef` in `HIServices/AXUIElement.h`), and reading it is a strict-
   concurrency error. The constant's literal value `"AXTrustedCheckOptionPrompt"` is used instead —
   it is the documented public name, not an implementation detail.
5. **`kern.boottime` moves without a reboot.** It is derived from the current clock, so an NTP
   correction shifts it by a second or two; a five-second tolerance keeps a clock adjustment from
   being logged as a restart. This is the Android `TamperLog.classifyConnect` lesson — a daemon
   that died and a machine that rebooted are different events — and `rebooted` is emitted first in
   the transition list so everything after it can be read against it.

**Still outstanding, and blocked on module 0:** a real grant, and a revocation observed through
`poll()`. Both need the stable signing identity module 0 produces. Running `permissions` from a
shell today reports the *terminal's* grants (the responsible-process rule), which the verb prints a
warning about; it is useful for the four environment signals, which were confirmed live, and for
nothing else. **The standard-user `tccutil` question in the verification section below is still
open** and is a separate matter from this module's code.

`assess()` implements the plan's recommended middle option — a self-admin user is reported as
`weakened`, never as `protected` — but it does **not** settle the product decision above. It makes
the two configurations distinguishable, which is the one outcome that must not be lost; what
onboarding *does* with that is still to be resolved in `content-interception.md`.

#### Reference documents

- [Apple — `CGPreflightScreenCaptureAccess`](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()) — the non-prompting check and its companion `CGRequestScreenCaptureAccess`.
- [Apple — `AXIsProcessTrustedWithOptions`](https://developer.apple.com/documentation/applicationservices/1462089-axisprocesstrustedwithoptions) — the Accessibility check and the prompt option key.
- [Apple — `IOHIDCheckAccess`](https://developer.apple.com/documentation/iokit/3181425-iohidcheckaccess) — Input Monitoring state, and the `IOHIDAccessType` values.
- [Apple — Protecting user privacy (TCC)](https://support.apple.com/guide/security/controlling-app-access-to-files-sec51e0c5d5b/web) — which permissions are admin-modifiable and which are user-revocable.

---

### 8. `ScreenCapture` — `ScreenCaptureKit`

```
Sources/MacDaemon/ScreenCapture.swift
```

**Built, except the live PNG-by-hand check — see the notes at the end of this section.**

Produces the same logical frame the Windows `capture` module produces, so the downstream scan
interface is shared:

```swift
struct CapturedFrame {
    let pixels: [UInt8]   // BGRA, row-major, tightly packed (no row padding)
    let width:  Int
    let height: Int
    let captured: Date
}
```

Responsibilities:

- Enumerate with `SCShareableContent`, build an `SCContentFilter`, run an `SCStream` with an
  `SCStreamOutput` delegate on a dedicated dispatch queue.
- Filter to the **frontmost window's application** rather than the whole display where possible, so
  the daemon's own overlay is not captured and re-scanned — a feedback loop that would otherwise
  re-trigger on the blurred backdrop it just drew. `SCContentFilter(display:excludingApplications:
  exceptingWindows:)` is the mechanism; excluding our own bundle identifier is not optional.
- Return an empty frame (zero width/height) when capture is unavailable, and let callers check —
  the same contract as the Windows module. Fail open.

Four traps, all of which produce plausible-looking wrong output rather than an error:

1. **`CVPixelBuffer` rows are padded.** `bytesPerRow` is aligned (commonly to 64 bytes) and is
   `≥ width * 4`. Copying `bytesPerRow * height` bytes and calling it a `width * height` image
   yields a progressively sheared frame that still decodes as an image and still scores as
   *something* on the classifier. Copy row by row using `CVPixelBufferGetBytesPerRow`, and lock with
   `CVPixelBufferLockBaseAddress(_:.readOnly)` first.
2. **The stream is change-driven, so a static image starves it.** `SCStreamFrameInfo.status` is one
   of `.complete`, `.idle`, `.blank`, `.suspended`; only `.complete` carries new pixels, and a
   motionless screen emits `.idle` frames with a nil surface. A still image is precisely the content
   this project most needs to catch, so the module must **retain the last `.complete` frame** and
   serve it to the scheduler on demand rather than requiring a fresh delivery per scan tick.
3. **`SCStreamConfiguration` defaults to point dimensions, not pixels.** On a Retina display that is
   half the real resolution, and the capture is a blurry downscale. Set `width`/`height` from the
   display's pixel dimensions and set `scalesToFit` deliberately. This interacts with the image
   model: `packages/image-sandbox` resizes the *shorter side* to 255 then centre-crops 224, so a
   wrong aspect ratio here silently changes what the model sees — the same class of error the
   `parity.rs` fixture caught on the Rust side.
4. **DRM-protected content captures as black.** Netflix, Apple TV+ and any HDCP-protected surface
   yield black frames by design. This is not a bug to fix; it is a coverage limit to record (below)
   and, more usefully, a detectable one — an all-black frame from a window that is playing video is
   a signal in itself.

`CGWindowListCreateImage` is obsoleted in macOS 15 and must not be used, including as a fallback.

#### What was built, and what is still outstanding

`ScreenCapture.swift` ships `CapturedFrame` as specified, plus `PixelBufferCopy.depad` (the
row-depadding fix for trap 1), `FrameDeliveryStatus` + `FrameCache` (the retention fix for trap 2,
kept independent of `ScreenCaptureKit` types so it is testable without linking the framework), and
`FrameAnalysis.isAllBlack` (the DRM-black-as-signal note, turned into a callable check rather than
left as a comment). `ScreenCapturing` is the edge protocol, mirroring `CommandRunner` and
`PermissionProbing`; `FakeScreenCapture` is its test double, and `SCShareableContentCapture` is the
real implementation — `SCShareableContent` → `SCContentFilter` (excluding our own bundle
identifier) → pixel-dimensioned `SCStreamConfiguration` → `SCStream`, with frames pushed through
`FrameCache` from the `SCStreamOutput` delegate callback. 20 tests, all on the pure logic (depad,
retention, black-detection); the edge is exercised only through `FakeScreenCapture` in tests, per
the `CommandRunner` pattern.

A `capture` CLI verb was added alongside `permissions` for live verification. Run from a shell
today it reaches the real `SCStream` setup and fails at the expected point — the process has no
Screen Recording grant — with `SCStreamErrorDomain Code=-3801`, confirming the request actually
reaches TCC rather than silently returning nothing. This is the same responsible-process gap
`PermissionGate` recorded: a real capture, and the PNG-by-hand check the plan calls for, are
**blocked on module 0's outstanding item** (a stable signing identity) and a human granting Screen
Recording to it, not on anything in this module.

#### Reference documents

- [Apple — ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) — `SCStream`, `SCShareableContent`, `SCContentFilter`, and the `SCStreamOutput` delegate contract.
- [Apple — `SCStreamFrameInfo.status`](https://developer.apple.com/documentation/screencapturekit/scstreamframeinfo) — the `.complete`/`.idle`/`.blank`/`.suspended` attachment that governs trap 2.
- [Apple — `CVPixelBuffer`](https://developer.apple.com/documentation/corevideo/cvpixelbuffer) — `CVPixelBufferGetBytesPerRow`, base-address locking, and the padding rule behind trap 1.
- [Apple — `CGWindowListCreateImage` deprecation](https://developer.apple.com/documentation/coregraphics/1455730-cgwindowlistcreateimage) — confirms the obsoletion this plan routes around.

---

### 9. `Scanner` and `ScanLoop` — the interface and the scheduler

```
Sources/MacDaemon/Scanner.swift
Sources/MacDaemon/ScanLoop.swift
```

Mirrors the Windows daemon's `scanner` + `scan_loop` split, and keeps the same vocabulary so the two
daemons stay comparable:

```swift
enum ScanAction { case allow, warn, block }
enum ScanSource { case ocr, image, text }
enum ProtectionMode { case full, warn, off }

struct NormalizedRect {
    let x: Double, y: Double, width: Double, height: Double  // each 0.0–1.0, frame-relative
}

struct ImageDetection {
    let label:      String
    let confidence: Double        // 0.0–1.0
    let box:        NormalizedRect
}

struct ScanVerdict {
    let action:     ScanAction   // effective action, after applying ProtectionMode
    let rawAction:  ScanAction   // as returned by the scanner, before the mode downgrade
    let score:      Double       // 0.0–1.0
    let source:     ScanSource
    let regions:    [ImageDetection]  // located image detections, empty for text-sourced
                                       // verdicts and for image verdicts where nothing
                                       // localized — see content-classification.md's
                                       // "Image Localization" section. An empty list on a
                                       // non-allow verdict means "cover the whole surface",
                                       // never "nothing to do".
}

protocol Scanner { func scan(_ frame: CapturedFrame) -> ScanVerdict }

struct NullScanner: Scanner { /* always .allow, empty regions — the only implementation at first */ }
```

The loop is the scheduler bridging `EventHooks` callbacks and the scanner:

- **Debounce** — discard events arriving within the debounce window of the previous one; only the
  last event in a burst produces a capture.
- **Cadence split**, per [edge-daemons.md](../../architecture/edge-daemons.md): the image classifier
  every ~500 ms while the surface is eligible; OCR every 1000–2000 ms, plus immediately after a
  foreground change or a meaningful frame difference. Text policy runs only when OCR returns
  meaningfully changed text.
- **Frame differencing** to avoid re-running OCR on an identical screen. Note this is *not* the
  same as trap 2 above: `SCStream` going idle tells you the screen did not change, which is exactly
  the cheap frame-difference signal the Windows daemon has to compute by hashing. Use it.
- **Skip** when there is no eligible surface, the display is asleep, or the session is locked.
- **Apply `ProtectionMode`** to produce `action` from `rawAction`, per
  [protection-modes.md](../../decisions/protection-modes.md): in `warn` mode a block-range score is
  downgraded to a warn; in `off` mode no scan runs at all.

`ScanLoop` must be **pure with respect to time** — `tick(now:)` takes the clock as a parameter, as
the Windows version does, so the debounce and cadence rules are unit-testable with an injected
clock and no capture at all. This is the module with the most testable logic in Layer 2 and it needs
tests first.

#### Where the mode→action decision should live — open

The mapping from *(score, thresholds, mode)* to an action is policy, and this would be its **third**
independent implementation: `packages/text-policy` has it in Rust, the Windows daemon plans it in
C++, and this would add Swift. Three implementations of a thresholding rule is three chances for the
platforms to disagree about what "block" means.

`packages/text-policy-ffi` already exposes `PolicyEngine`/`evaluate` over UniFFI, and **UniFFI
generates Swift bindings natively** — Swift is a first-class target, unlike the Kotlin path that
needed the `apps/mobile/scripts/build-ffi.sh` scaffolding. So the cheap correct answer is to consume
the existing Rust policy from Swift rather than reimplement it, and this is the first platform where
that costs almost nothing.

Not decided here because it affects the Windows daemon too, and because it adds a Rust build step to
a package that currently has none. Flagging it so the choice is made once, deliberately, rather than
by default at implementation time.

#### Where image localization should live — open

`ScanVerdict.regions` is the interface the rest of Layer 2 (`Overlay`, `DaemonIPC`) is written
against, but nothing today populates it beyond an empty list. Per
[content-classification.md](../../architecture/content-classification.md#image-localization), the
target is a real detector model; until `machine-learning` has one, the pragmatic first
implementation is the coarse-grid fallback described there, run inside the real `Scanner`
implementation that eventually replaces `NullScanner` — this module's job is only to make sure the
type carries the information once it exists, not to implement the detection itself.

---

### 10. `Overlay` — borderless `NSWindow`

```
Sources/MacDaemon/Overlay.swift
```

The response surface. Two distinct visual states, and the sketch conflated them:

| State | Mouse events | Purpose |
|---|---|---|
| Warn interstitial | **Must be swallowed** | Blur + verse + a deliberate choice, per [warn-interstitial.md](../../product/flows/warn-interstitial.md) |
| Block cover | **Must be swallowed** | Full interrupt, per [block.md](../../product/flows/block.md) |
| Pre-blur / passive | `ignoresMouseEvents = true` | A hint that does not interrupt |

`ScanVerdict.regions` (module 9) makes a fourth, narrower state possible — a cover limited to the
detected `NormalizedRect`s instead of the whole window or display — but treat full-surface cover as
the only implemented behavior for now. A region cover needs the window's on-screen bounds *at
capture time* to convert a frame-normalized box into screen coordinates, and the window can move or
resize between capture and render; get the simple case (whole-surface cover, ignoring `regions`)
correct and verified first, and revisit region-level cover once `Scanner` actually populates
`regions` instead of always returning an empty list.

`ignoresMouseEvents` belongs to the passive case only. An interstitial that lets clicks through to
the content underneath is not an interstitial — the user can keep interacting with exactly what was
just flagged.

Requirements:

- Transparent, borderless, `NSWindow.Level` above normal windows; `.screenSaver` level clears the
  menu bar and Dock.
- `collectionBehavior` must contain **both** `.canJoinAllSpaces` **and** `.fullScreenAuxiliary`.
  With only one, the overlay silently fails to appear over native-fullscreen video — which is
  precisely the case that matters most, and it fails by simply not showing rather than by erroring.
- **One overlay per `NSScreen`.** A single window covers one display; on a two-monitor setup the
  second screen is left showing the content. Observe
  `NSApplication.didChangeScreenParametersNotification` and rebuild the set when displays are
  connected, disconnected, or rearranged.
- **The process needs an `NSApplication` run loop.** This package is currently a plain SwiftPM
  executable with no `NSApplication`, and an `NSWindow` created without one will never appear.
  Layer 2's GUI-session half must call `NSApplication.shared`, set
  `setActivationPolicy(.accessory)` — so it draws overlays without a Dock icon or menu bar — and run
  the main run loop. This is another reason Layer 1 and Layer 2 are separate processes.
- The blurred backdrop uses the captured frame per
  [warn-interstitial.md](../../product/flows/warn-interstitial.md). **That frame is never persisted,
  never logged, and never leaves the device** — it is a texture, and the local-first rule in the
  root `AGENTS.md` applies to it with full force.

Keep the drawing thin: the pure part is *which* overlays should exist for a given verdict and screen
configuration, and that is a testable function returning a set of frames. AppKit does the rest.

#### Reference documents

- [Apple — `NSWindow.CollectionBehavior`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior) — `.canJoinAllSpaces` and `.fullScreenAuxiliary` semantics for overlays over fullscreen Spaces.
- [Apple — `NSWindow.Level`](https://developer.apple.com/documentation/appkit/nswindow/level) — the ordering constants including `.screenSaver`.
- [Apple — `NSApplication.ActivationPolicy`](https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy) — `.accessory` for a UI-bearing process with no Dock presence.

---

### 11. `EventHooks` — window lifecycle and scroll

```
Sources/MacDaemon/EventHooks.swift
```

Supplies the fast wakeups the scan loop debounces, per
[edge-daemons.md](../../architecture/edge-daemons.md). Two sources:

- **`AXObserver`** for window lifecycle and geometry —
  `kAXFocusedWindowChangedNotification`, `kAXWindowMovedNotification`,
  `kAXWindowResizedNotification`, plus `NSWorkspace.didActivateApplicationNotification` for the
  application-level change. Requires **Accessibility**. An `AXObserver` is inert until its run-loop
  source is added via `CFRunLoopAddSource(_, AXObserverGetRunLoopSource(observer), .defaultMode)` —
  omitting that yields an observer that registers successfully and never fires.
- **Scroll**, feeding the scroll-delta debounce.

**On scroll, prefer `NSEvent.addGlobalMonitorForEvents(matching: .scrollWheel)` over `CGEventTap`.**
The daemon only needs to *observe* scroll, never to consume or modify it, and the global monitor is
the read-only API for exactly that. It avoids the `CGEventTap` failure mode that is missed almost
universally: **the system disables a tap that takes too long to respond**, delivering a
`kCGEventTapDisabledByTimeout` event, and unless the callback watches for that event type and calls
`CGEventTapEnable` again, scroll detection dies silently mid-session and never recovers. If a tap
turns out to be necessary anyway, that re-enable path is mandatory, not defensive.

The monitor route also likely avoids requesting **Input Monitoring** at all. Every additional TCC
prompt is onboarding friction on a permission the user can revoke, so the fewer requested the
better. Which permission a listen-only scroll monitor actually requires on macOS 26 should be
**measured on the real system before it is written into onboarding** — the documentation is not
crisp on the Accessibility/Input Monitoring boundary for non-keyboard events, and this plan should
not assert what it has not run.

#### Reference documents

- [Apple — Accessibility API / `AXUIElement`](https://developer.apple.com/documentation/applicationservices/axuielement_h) — `AXObserver` creation, notification registration, and run-loop source attachment.
- [Apple — `NSEvent` global monitors](https://developer.apple.com/documentation/appkit/nsevent/1535472-addglobalmonitorforevents) — the read-only observation path preferred here.
- [Apple — Quartz Event Services (`CGEventTap`)](https://developer.apple.com/documentation/coregraphics/quartz_event_services) — tap creation, the timeout-disable behaviour, and `CGEventTapEnable`.

---

### 12. `AccessibilityText` — AX text extraction

```
Sources/MacDaemon/AccessibilityText.swift
```

Reads on-screen text without injection by walking the AX tree of the focused window
(`kAXValueAttribute`, `kAXTitleAttribute`, `kAXDescriptionAttribute` over the descendant elements),
and feeds it to `packages/text-policy`. Behind **Accessibility**.

A **supplement to OCR, not a replacement.** Coverage varies wildly, and anything drawing its own
text — canvas-based UIs, games, most Electron content — returns nothing useful. The scan loop must
never treat an empty AX result as "no text on screen".

Two traps:

1. **AX calls are synchronous cross-process IPC and can block for seconds** against a hung or busy
   application. Called on the scan loop's thread, one unresponsive app stalls the entire daemon.
   Call `AXUIElementSetMessagingTimeout` with a short timeout (a few hundred ms) and do the walk off
   the scheduler's thread. Also bound the tree walk by depth and node count — some apps expose
   pathologically large trees.
2. **Chromium-based apps expose nothing by default.** Chrome, Electron and anything else on
   Chromium keep their accessibility tree switched off for performance and only build it when a
   client sets `AXManualAccessibility` to true on the application element. Without that, the walk
   returns a nearly empty tree and reads as "this app has no text" — which, given how much of the
   relevant surface is a browser, would quietly gut the module's value.

#### Reference documents

- [Apple — `AXUIElement` attributes](https://developer.apple.com/documentation/applicationservices/axuielement_h) — attribute reads and `AXUIElementSetMessagingTimeout`.
- [Chromium — accessibility on macOS](https://www.chromium.org/developers/design-documents/accessibility/) — the on-demand tree and the `AXManualAccessibility` opt-in behind trap 2.

---

### 13. `FullscreenControl` — AX `AXFullScreen`

```
Sources/MacDaemon/FullscreenControl.swift
```

Force-exit fullscreen by setting `kAXFullScreenAttribute` to `false` on the focused window element.
Behind **Accessibility**.

Keep it as the **fallback path only**. A correctly configured overlay (module 10, with both
collection-behaviour flags) already covers a fullscreen Space, and yanking the user out of fullscreen
is a far more violent interaction than covering the screen. Reach for this when the overlay is known
not to work — the clearest case being applications that take an exclusive display capture, where no
window can be composited above them and covering is impossible by construction.

---

### 14. `DaemonIPC` — carrying verdicts to the desktop app

```
Sources/MacDaemon/DaemonIPC.swift
```

The macOS counterpart to the Windows daemon's named-pipe server. Every flow in
[block.md](../../product/flows/block.md) and
[warn-interstitial.md](../../product/flows/warn-interstitial.md) ends with the daemon emitting a
`scan_event` that the Electron app records, and
[protection-mode-change.md](../../product/flows/protection-mode-change.md) sends a `config_update`
back the other way — none of which is possible without this module.

- Transport: a **Unix domain socket** in the user's container, not a TCP port. A localhost port is
  reachable by every process on the machine including a browser page, which for a control channel
  that can set `protection_mode` is a bypass.
- Message shapes match the Windows daemon's so `daemon-ipc.ts` needs one transport adapter rather
  than a second protocol: `scan_event { action, score, source, ts, regions }` outbound,
  `config_update { block_threshold, warn_threshold, protection_mode }` inbound. `regions` is
  `ScanVerdict.regions` (module 9) serialized as normalized rects and is an empty array until a real
  detector or the coarse-grid fallback populates it — see
  [content-classification.md](../../architecture/content-classification.md#image-localization). Keep
  the field in the shared shape now, even though only macOS produces it today, so the Windows daemon
  does not have to break protocol compatibility to add it later.
- The warn path additionally carries the captured frame for the blurred backdrop. Given the size of
  a Retina frame, encode as JPEG rather than raw BGRA, and hold to the same rule as the overlay: it
  is never written to disk and never logged.
- Framing, parsing and message validation are pure and get tests first; the socket is the edge.

---

### 15. `SettingsGuard` — notice the settings pane, and record it

```
Sources/MacDaemon/SettingsGuard.swift
```

The macOS counterpart to `apps/mobile`'s `SettingsGuard`, with a deliberately smaller remit. Detects
that System Settings has come forward, records it, and optionally shows the interstitial.

**It costs no permissions.** Verified on macOS 26.5.2 with both grants off:
`NSWorkspace.shared.frontmostApplication` returns bundle identifier, name and PID, so watching for
`com.apple.systempreferences` via `NSWorkspace.didActivateApplicationNotification` needs no TCC at
all. Detecting the *pane* is what costs a grant — of 21 on-screen windows,
`CGWindowListCopyWindowInfo` gave owner names and bounds for all and a non-empty `kCGWindowName` for
**none**, because titles are redacted without Screen Recording.

The general rule this module exists to respect: **capture classifies what is inside a window;
identifying which window is metadata and is cheap.** Never run OCR to learn something a bundle
identifier already answers.

Response options, in order of preference:

1. **Record to the tamper log** and, in partnered mode, notify. This is the primary behaviour.
2. **Show the interstitial** (module 10) — a verse and a pause, which is the product's actual
   mechanism.
3. Cover or `forceTerminate()` — available without TCC, but **rank it as friction, not a lock**, and
   prefer not to. A process that force-quits System Settings behaves like malware, races the user,
   and is a strong candidate for being the reason the product gets uninstalled.

**Do not model this on the Android version's importance.** There, the accessibility screen is the
only door a normal user has, which is what makes `GLOBAL_ACTION_BACK` load-bearing. On macOS the
user has a terminal, so `tccutil`, `launchctl`, `kill` and Recovery all remain open — and
`PermissionGate`'s revocation polling detects the *outcome* of all of them, including this one. This
module is defence in depth on top of that, never a substitute for it.

---

## What Layer 2 does *not* cover on macOS — honest limits

As with Layer 1, these are inherent to the mechanism and are recorded here rather than discovered
later:

- **DRM-protected video captures as black.** HDCP-protected surfaces (Netflix, Apple TV+, and
  others) are excluded from `ScreenCaptureKit` by design. Streaming services in that category are
  invisible to the image model.
- **Exclusive-display applications cannot be covered.** Where a game or app takes exclusive control
  of the display, no window composites above it. Module 13 is the only lever, and it does not always
  apply.
- **Screen Recording is permanently revocable.** Apple reserves this from MDM specifically so it
  always remains user-consentable, which also means it always remains user-*revocable* by anyone who
  can authenticate as an admin. The mitigation is detection and reporting (module 7), never
  prevention.
- **Nothing is captured while the session is locked or the display is asleep** — correct behaviour,
  but it means the daemon's coverage is not continuous and the event log will have gaps that are not
  evidence of anything.
- **The protected user being a local admin defeats the whole model**, Layer 1 and Layer 2 together.
  See module 7; this is a setup requirement, not something code can fix.
- **The capture path sees everything** — banking sessions, password managers, private messages. This
  is the single largest privacy surface in the project. Frames stay in memory, are never persisted,
  never logged, and never leave the device; the only frame that crosses a process boundary is the
  warn-interstitial backdrop over the local socket. Any future change here needs an explicit
  decision, not an implementation detail.

## Layer 2 implementation order

0. ~~**Module 0 — the signed bundle and the `launchd` split.** Nothing below can be honestly
   verified before this exists, because every grant made to an unsigned binary is invalidated by
   the next build, and every grant made from a terminal belongs to the terminal.~~ **Done**, with
   one decision outstanding: the bundle signs ad-hoc until a stable certificate exists, and both
   `bundle` and `agent` say so at runtime. See the module 0 section above for the five findings,
   the first of which contradicts the instruction the section was written with.
1. **`PermissionGate`** (module 7) — ~~pure snapshot logic and transitions under test, then the
   three preflight calls~~ **Done**, ahead of module 0 because none of it needs a grant — then a
   real grant against the signed bundle. Verify revocation is *detected*, not just that the grant
   works. **That last part is still outstanding and is blocked on module 0**: revocation can only
   be observed against a stable signing identity, and today `permissions` run from a shell reports
   the *terminal's* grants. The pure half is 38 tests (`PermissionGateTests.swift`), and all four
   zero-permission environment signals were confirmed live through the `permissions` verb.
2. **`ScreenCapture`** (module 8) — ~~get one `.complete` frame out of `SCStream` and write it to a
   PNG by hand before building anything on top of it. Confirm the row-padding copy against a known
   test pattern rather than by eye; a sheared frame looks fine at a glance.~~ **Done**, except the
   live PNG-by-hand check, which is blocked on the same real grant as `PermissionGate` — see the
   module 8 section above.
3. **`Scanner` + `ScanLoop`** (module 9) with `NullScanner` — ~~the debounce, cadence and mode logic
   are the most testable code in Layer 2 and need no permissions at all. This step can proceed in
   parallel with steps 1–2 by anyone blocked on grants.~~ **Done.** `ScanVerdict` carries the
   `regions: [ImageDetection]` field per
   [content-classification.md](../../architecture/content-classification.md#image-localization), but
   nothing populates it yet — `NullScanner` and every test double return an empty list, per that
   section's deferral of real image localization. The mode→action mapping is plain Swift, not
   UniFFI/`text-policy-ffi` — see module 9's "open" note, still undecided. 62 new tests
   (`ScannerTests.swift`, `ScanLoopTests.swift`).
4. **`Overlay`** (module 10) — the pure part is **done**: `OverlayPlan.plan(intent:screens:)` decides
   one placement per screen from a verdict-shaped intent, with `.screenSaver`-equivalent level and
   `.canJoinAllSpaces`/`.fullScreenAuxiliary`-equivalent collection behavior recorded as data for a
   later AppKit task to consume. Region-level cover is deliberately not implemented — every placement
   still covers the full screen frame, per the `Overlay` section's note above. Still outstanding and
   unchanged from before: real `NSWindow`/`NSApplication` construction (needs the run loop this
   package doesn't have yet), and live verification over native-fullscreen video and on a
   multi-display setup — both failure modes are silent and neither can be checked without the AppKit
   half. 16 new tests (`OverlayTests.swift`).
5. **`DaemonIPC`** (module 14) — the pure part is **done**: length-prefixed JSON framing, an
   incremental decoder (`.needsMoreData` / `.message` / `.oversized` / `.invalid`, never throwing on
   a partial read), and validated `scan_event`/`config_update` Codable types, with `regions`
   (`ImageDetection`, always an array, never omitted) on `scan_event` per the module 14 section above.
   34 new tests (`DaemonIPCTests.swift`). Still outstanding: the actual Unix domain socket
   transport, and wiring into `apps/desktop/src/main/daemon-ipc.ts`, whose current shape
   (`verdict`/`windowTitle`/`at`, camelCase) does not yet match this wire protocol. Also flagged but
   not resolved: `ts` is encoded here as a Double (Unix epoch seconds), while the Windows daemon plan
   documents an ISO-8601 string — a cross-platform choice to make once, deliberately, not by default.
6. **`EventHooks`** (module 11) — measure which permission the scroll monitor actually requires
   before writing onboarding copy.
7. **`AccessibilityText`** (module 12) — validate against a Chromium-based app early, since that is
   where the `AXManualAccessibility` trap bites and where the coverage question is decided.
8. **`FullscreenControl`** (module 13) — last, and only if module 10 proves insufficient in practice.
9. **`SettingsGuard`** (module 15) — can be built at any point after step 1, since it needs no
   permissions; sequenced last because it is defence in depth over `PermissionGate`, not a
   substitute for it.

### Outstanding verification — one item blocks a tamper-resistance claim

**Can a *standard* user reset a Screen Recording grant with `tccutil`?** Measured so far: `tccutil
reset ScreenCapture <bundle-id>` runs without `sudo` and fails only at bundle-ID resolution
(`OSStatus -10814`) — but it was run from an **admin** account, since the development machine's user
is in the `admin` group. Screen Recording lives in the system-wide TCC store rather than the per-user
one, so it plausibly fails for a standard user; "plausibly" is doing far too much work for the
assumption the entire account model rests on.

This needs a real standard-user account on a Mac and takes minutes to settle. **No tamper-resistance
claim should ship before it is run**, because if a standard user *can* run it, the standard-user
configuration is worth much less than the model above assumes and the plan needs revisiting.

Real classifiers replace `NullScanner` once the pipeline is proven end to end; `packages/image-sandbox`
is the image side and already runs under `mitm-proxy`, so the work is binding it to a frame rather
than building it.

---

## What this package does not cover

- **Classification** — text-policy, OCR, and image ML stay in their own packages. The daemon calls
  them; it does not implement them.
- **TLS interception** — `packages/mitm-proxy` owns it. The daemon only installs trust and routes
  traffic.
- **Safari extension** — Safari Web Extensions must ship inside a notarized App Store app. Safari
  coverage leans on the proxy alone for now; revisit only if the project ships a Mac App Store
  build.
- **Process injection** — impossible on macOS (SIP ignores `DYLD_INSERT_LIBRARIES` for Apple
  binaries; the hardened runtime blocks it for notarized third-party apps) and not in the core
  model anyway.
- **iOS** — a separate, lesser product. See the iOS section of
  [content-interception.md](../../decisions/content-interception.md).
