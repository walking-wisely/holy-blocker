# Security Backlog

This document turns the current security discussion into a concrete backlog for Holy Blocker.
It is intentionally scoped to the architecture that exists or is already planned in this
repository: a local-first desktop app, Windows daemon, local MITM proxy, policy engine,
and local ML pipeline.

The backlog is ordered by engineering priority, not by abstract severity labels. Items in
`Now` are the controls that should be established early because they reduce risk across the
entire repository and release process. Items in `Soon` depend on more runtime code existing.
Items in `Later` are important, but they should follow once the relevant components stabilize.

## Security objectives

- Preserve the local-first model: no screenshots, OCR text, browsing history, or decrypted
  traffic should leave the device unless a future feature explicitly requires it.
- Prevent silent disablement: local processes and local users should not be able to turn off
  protection through weak IPC, weak file permissions, or unsafe defaults.
- Prevent secret leakage: CA private keys, signing credentials, private eval packs, and model
  artifacts must not leak through the repo, CI logs, or public workflows.
- Keep security decisions explainable: policy results, mode changes, and override attempts
  should be auditable without logging raw sensitive content.

## Now

### 1. Branch protection and required checks

Add branch protection for the default branch (`master`) and require passing status checks before merge.

Why now:

- This is the lowest-cost way to prevent bypassing future security checks.
- It creates a single place to enforce typecheck, tests, and security scans.

Suggested checks:

- JavaScript workspace install/build/typecheck
- Rust tests
- Python tests
- Native Windows build/test job
- Secret scanning
- SAST

### 2. Secret scanning in CI

Add repository-wide secret scanning with `gitleaks` or `trufflehog`.

Why now:

- This project will eventually handle CA material, private eval-pack credentials, and signing
  secrets.
- Secret scanning is valuable before those assets exist in the repo because it sets the guardrail
  early.

Definition of done:

- Pull requests fail on detected secrets.
- Baseline/allowlist handling is documented so false positives do not train maintainers to ignore
  alerts.

### 3. CodeQL SAST across TypeScript, Rust, Python, and C++

Enable GitHub CodeQL for all supported languages in this repository.

Why now:

- The repo is polyglot and crosses high-risk trust boundaries.
- It is more useful to start with broad language coverage than to overfit one package.

Focus areas:

- Electron IPC and preload exposure
- Unsafe filesystem handling
- Rust parsing and proxy input handling
- C++ memory and handle lifetime issues
- Python artifact loading and shelling/conversion flows

### 4. Dependency update and supply-chain hygiene

Enable Dependabot or Renovate for `pnpm`, Cargo, Python, and GitHub Actions.

Why now:

- Electron, Rust TLS crates, Python ML tooling, and test dependencies will age quickly.
- GitHub Actions are part of the supply chain and should be pinned and updated deliberately.

Definition of done:

- Lockfiles are committed and reviewed.
- Action versions are pinned to immutable references where practical.
- Update PRs run the normal CI checks.

### 5. Public vs private CI separation

Define which jobs are safe for untrusted pull requests and which are trusted-only.

Why now:

- The repo already expects private eval packs and sensitive artifacts later.
- If the trust model is not defined early, secrets tend to leak into generic workflows.

Public CI:

- Build, typecheck, unit tests, sanitized fixtures, static scans

Trusted-only CI:

- Private eval-pack download
- Signing
- Release packaging
- Any job that touches secrets, private models, or private corpora

### 6. SECURITY.md and disclosure process

Add a top-level `SECURITY.md`.

Why now:

- Low effort and appropriate for an OSS security-sensitive project.
- Gives researchers and users a reporting channel before releases grow.

Keep it simple:

- Supported versions
- Reporting contact
- What not to include in public issues
- Expected response pattern

### 7. STRIDE threat models for current trust boundaries

Create initial threat models for the two main runtime paths:

- screen-capture path
- network interception path

Why now:

- The risk in Holy Blocker is not generic web risk; it is local trust-boundary failure.
- STRIDE should drive design controls, tests, and CI policy rather than sit as a standalone doc.

Minimum threat-model scope:

- renderer -> preload -> Electron main
- Electron main -> named pipe -> Windows daemon
- daemon -> local storage
- browser/client -> MITM proxy -> origin
- proxy -> CA key material
- CI -> private eval assets -> release artifacts

## Soon

### 8. Named-pipe hardening and local peer trust

Harden desktop/daemon IPC before the named-pipe protocol becomes a control plane.

Why soon:

- The desktop plan already introduces persistent named-pipe IPC and config updates.
- A forged local client/server is one of the clearest spoofing risks in the repo.

Controls to implement:

- Restrictive pipe ACLs
- Explicit server/client ownership expectations
- Defensive message validation
- Versioned message schema
- Tests for malformed, oversized, replayed, or unauthorized messages

### 9. File-permission and secret-material policy

Define how local sensitive files are stored and protected.

Applies to:

- `data/ca/`
- local policy/config files
- stats/event stores
- future private model artifacts
- downloaded private eval packs on trusted runners

Controls:

- keep secrets out of the repo by default
- document allowed locations
- minimize retained sensitive content
- set restrictive filesystem permissions where the platform allows it

### 10. Sensitive logging and redaction rules

Add explicit rules for what may never appear in logs, test output, or CI artifacts.

Must not log by default:

- screenshots
- OCR text
- decrypted HTTP bodies
- raw private eval cases
- root CA private key material

Preferred alternatives:

- opaque case IDs
- aggregate metrics
- hashes or redacted summaries
- structured event types without raw content

### 11. Release artifact signing and provenance

Define the release trust chain before end-user distribution starts.

Why soon:

- This is a local security product. Users need confidence that binaries, native modules, and
  model artifacts are authentic.

Scope:

- desktop binaries/installers
- native daemon binaries
- proxy binaries
- shipped model artifacts

Controls:

- signing for release artifacts
- published checksums
- provenance or attestation from trusted CI

### 12. Security-focused test cases for abuse paths

Turn the highest-value STRIDE findings into tests.

Examples:

- forged `config_update` attempts
- malformed IPC message parsing
- proxy parser stress cases
- oversized body buffering limits
- config tampering and invalid threshold files
- model artifact checksum mismatch

The goal is not exhaustive pentesting. The goal is preventing silent regressions on known abuse
paths.

### 13. SBOM generation for releases

Generate a software bill of materials for release artifacts.

Why soon:

- The repo is polyglot and will ship security-relevant binaries.
- SBOMs make dependency review and incident response easier when a library vulnerability lands.

## Later

### 14. DAST for any future hosted surfaces

If the project later adds hosted services, admin panels, docs apps, or APIs, add DAST such as
OWASP ZAP against staging.

Why later:

- The current core product is local-first, not an internet-facing SaaS app.
- DAST is lower value than IPC, artifact, and local-storage hardening at the current stage.

### 15. Infrastructure-as-code scanning

If the project introduces Terraform, Pulumi, or cloud deployment manifests, add IaC scanning
with tools such as Checkov or Trivy.

Why later:

- This only becomes relevant once cloud infrastructure exists.
- It should be added immediately when infrastructure code appears, not before.

### 16. Fuzzing and property-based testing on parsers and scanners

Add fuzzing where untrusted input is parsed or normalized.

Strong candidates:

- proxy request/response handling
- TLS ClientHello/SNI parsing
- text normalization and matcher inputs
- IPC framing and JSON decoding

### 17. Reproducible-build improvements

Improve the ability to reproduce release artifacts from source.

Why later:

- Valuable for trust and incident response.
- Usually easier after the packaging pipeline stabilizes.

### 18. Incident response and key rotation playbooks

Write operational runbooks for:

- leaked signing credentials
- leaked CA material
- compromised private eval storage
- malicious dependency introduction

These do not need to be elaborate at first, but they should exist before external distribution
becomes broad.

## Recommended rollout order

If only a few items can be started immediately, do them in this order:

1. Branch protection and required checks
2. Secret scanning
3. CodeQL
4. Dependency update automation
5. Public vs private CI separation
6. SECURITY.md
7. Initial STRIDE documents

That sequence gives the repository a real baseline without waiting for all runtime components to
be finished.
