# 2. `CATrust` — root CA installation in the System keychain

Part of the [macOS Daemon plan](../plan.md).

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

## Reference documents

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
