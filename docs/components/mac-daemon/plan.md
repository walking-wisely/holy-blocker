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

The current machine has **Swift 6.3 with Command Line Tools only — no full Xcode**. This is
sufficient for all of Layer 1 (a plain SwiftPM executable, no app bundle, no entitlements). Layer 2
will eventually need a signed `.app` bundle for stable TCC permission grants, since TCC identifies
clients by code signature and unsigned command-line binaries get inconsistent, easily-invalidated
grants. Treat "install Xcode and produce a signed bundle" as a prerequisite of Layer 2 step 5, not
of anything before it.

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
— but every mechanism is an Apple API behind a TCC permission. **Do not start Layer 2 until Layer 1
is working end to end**; Layer 1 is nearly free given the existing proxy, and Layer 2 is where the
real cost is.

Sketched here for ordering purposes; each module gets a full specification when Layer 1 lands.

### 7. `ScreenCapture` — `ScreenCaptureKit`

`SCStream` against `SCShareableContent`. Requires the **Screen Recording** permission.
`CGWindowListCreateImage` is obsoleted in macOS 15 and must not be used. Produces the same frame
type the Windows `capture` module produces so the downstream scan interface is shared.

### 8. `Overlay` — borderless `NSWindow`

Transparent, high `windowLevel`, `ignoresMouseEvents` for the pre-blur case. Covering a
native-fullscreen Space needs `collectionBehavior` of `.canJoinAllSpaces` and `.fullScreenAuxiliary`
— without both, the overlay silently fails to appear over fullscreen video, which is precisely the
case that matters most.

### 9. `EventHooks` — `AXObserver` and `CGEventTap`

`AXObserver` for window lifecycle and geometry, `CGEventTap` for scroll (feeding the scroll-delta
debounce). Behind **Accessibility** and **Input Monitoring** permissions respectively.

### 10. `AccessibilityText` — AX text extraction

Reads on-screen text without injection via the AX API. A **supplement** to OCR where it works, not a
replacement — coverage varies wildly across apps, and any app drawing its own text (Electron, games,
canvas-based UIs) returns nothing useful.

### 11. `FullscreenControl` — AX `AXFullScreen`

Force-exit fullscreen via the AX attribute. Usually unnecessary given a Space-covering overlay; keep
it as the fallback path.

### 12. `PermissionGate` — TCC state and the tamper model

Detect which permissions are granted, guide the admin through granting them once, and — critically —
**detect when the protected user is themselves a local admin**, which silently defeats the entire
tamper model. This is an open question in
[content-interception.md](../../decisions/content-interception.md) and needs a product decision
before onboarding is written.

#### Reference documents

- [Apple — ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) — `SCStream`,
  `SCShareableContent`, `SCContentFilter`, and the `SCStreamOutput` delegate contract.
- [Apple — `CGWindowListCreateImage` deprecation](https://developer.apple.com/documentation/coregraphics/1455730-cgwindowlistcreateimage) — confirms the obsoletion this plan routes around.
- [Apple — Accessibility API / `AXUIElement`](https://developer.apple.com/documentation/applicationservices/axuielement_h) — `AXObserver` creation, notification registration, and attribute reads including `kAXFullScreenAttribute`.
- [Apple — `CGEventTap`](https://developer.apple.com/documentation/coregraphics/quartz_event_services) — event tap creation, the Input Monitoring requirement, and tap re-enabling after timeout.
- [Apple — `NSWindow.CollectionBehavior`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior) — `.canJoinAllSpaces` and `.fullScreenAuxiliary` semantics for overlays over fullscreen Spaces.
- [Apple — Protecting user privacy (TCC)](https://support.apple.com/guide/security/controlling-app-access-to-files-sec51e0c5d5b/web) — which permissions are admin-modifiable and which are user-revocable. The asymmetry that Screen Recording **cannot** be force-granted by MDM is the load-bearing fact behind the account-based tamper model.

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
