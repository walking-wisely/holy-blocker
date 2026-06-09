# Security Policy

Holy Blocker is a local-first, on-device content blocker. It runs with elevated
privileges (a Windows daemon, a local MITM proxy with its own certificate
authority, and a network filter), so the security of its trust boundaries
matters more than for a typical application. We take vulnerability reports
seriously and appreciate responsible disclosure.

## Supported versions

The project is pre-release and under active development. Security fixes are
applied to the latest `master` and will be applied to released versions once a
release process exists.

| Version | Supported |
| ------- | --------- |
| `master` (latest) | ✅ |
| Tagged releases | None yet |

This table will be updated when versioned releases begin.

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report privately through either channel:

1. **GitHub private vulnerability reporting** (preferred) — open the
   [Security tab](https://github.com/walking-wisely/holy-blocker/security)
   and click **Report a vulnerability**. This keeps the report private and lets
   us collaborate on a fix and advisory in one place.
2. **Email** — `help@holy-blocker.com`. Please put `holy-blocker security` in
   the subject line.

### What to include

A useful report usually has:

- a description of the issue and the trust boundary it crosses;
- steps to reproduce, ideally with a minimal proof of concept;
- the impact you believe it has (e.g. silent disablement of protection,
  secret leakage, local privilege escalation);
- affected component, OS version, and build/commit if known.

**Do not include real secrets, private datasets, screenshots of sensitive
content, or CA private key material in your report.** A redacted summary,
opaque identifiers, or hashes are sufficient to demonstrate an issue.

## Scope

Because Holy Blocker is local-first, its highest-value attack surfaces are
local trust boundaries rather than remote network exploits. Reports touching
these areas are especially valuable:

- **CA key material** — extraction or misuse of the local proxy's certificate
  authority private key.
- **IPC bypass / spoofing** — forging or hijacking the desktop ⇄ daemon
  named-pipe channel to change protection mode or config.
- **Silent disablement** — turning off protection through weak file
  permissions, unsafe defaults, or unauthenticated local control paths.
- **Privilege escalation** — abusing the Windows daemon or network filter to
  gain privileges beyond their intended scope.
- **Secret leakage** — CA keys, signing credentials, private eval packs, or
  model artifacts leaking through the repo, logs, or CI.

### Out of scope

The following are generally **not** considered vulnerabilities:

- Issues that require an attacker to already have administrator/SYSTEM access
  on the device, since that level of access defeats any on-device control.
- The locally generated CA certificate being trusted on the device it was
  generated for — this is by design and required for MITM inspection.
- The product not blocking a specific piece of content (a classification miss
  is a quality issue, not a security vulnerability — please file it as a
  regular issue without sensitive samples).

## Response expectations

- We aim to **acknowledge** a report within **48 hours**.
- We aim to provide an initial **triage assessment** within **7 days**.
- We will keep you informed as we work on a fix and will credit you in the
  advisory unless you prefer to remain anonymous.

Please give us a reasonable opportunity to release a fix before any public
disclosure.
