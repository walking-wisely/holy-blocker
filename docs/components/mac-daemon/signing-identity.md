# macOS daemon — signing identity runbook

`HolyBlockerDaemon.app` must be signed with a **stable** code-signing identity before any TCC grant
(Screen Recording, Accessibility, Input Monitoring) is worth making. This page is the operational
reference: where the current identity lives, how to recreate it if it's lost, and how to rotate it
deliberately. The reasoning for *why* this is a prerequisite lives in
[plan.md](plan.md) (module 0); this page is just the "the cert died, now what" runbook.

## Why it has to be stable

TCC keys a permission grant to the app's **code-signing identity**, not its path or its contents.

- **Unsigned** doesn't really happen — Apple Silicon won't execute an unsigned Mach-O at all.
- **Ad-hoc** (`codesign`'s default with no `-sign` identity) derives the identity from the binary's
  `cdhash`. Every `swift build` changes the binary, which changes the cdhash, which makes TCC treat
  it as a brand-new, never-granted client. A grant obtained on Monday is gone by Tuesday's build.
- **Signed with a certificate** embeds the certificate's identity, not a hash of the binary. Sign
  the same bundle with the same certificate before and after a rebuild and TCC sees the same client
  both times.

`CodeSigning.SigningIdentity.isStable` in `Sources/MacDaemon/CodeSigning.swift` is exactly this
question, and `holy-blocker-macd bundle-status` reports it as "grants survive a rebuild: true/false".

## Current identity

- **Name:** `Holy Blocker Dev`
- **Type:** self-signed, `codeSigning` extended key usage, 10-year validity
- **Location:** the developer's **login keychain**
  (`~/Library/Keychains/login.keychain-db`) — machine-local, never committed to the repo
- **Scope:** development only. It is trusted for code-signing purposes on this machine alone; it
  will not pass Gatekeeper or notarization and must never be used for anything distributed to
  another machine. See "Before shipping to anyone else" below for what that actually requires.

Inspect it at any time:

```bash
security find-identity -v -p codesigning
```

Should print a line containing `"Holy Blocker Dev"`. If that line is missing, the identity was
never created on this machine, was deleted, or the keychain was reset (new machine, OS reinstall,
`security delete-identity`, etc.) — go to "Recreating it" below.

## Recreating it (identity lost)

Losing the identity is expected on a fresh machine or after a keychain reset — this certificate is
never backed up outside the local keychain by design (see "Why it isn't backed up" below). Run:

```bash
cd native-modules/mac-daemon
scripts/create-dev-signing-identity.sh
```

It's idempotent: if `Holy Blocker Dev` already exists it prints that and does nothing. If it's
missing, it generates a fresh self-signed certificate, imports it into the login keychain, and
trusts it for code signing — the same three `openssl`/`security` steps this was originally set up
with by hand.

Then re-point the build at it and re-sign:

```bash
export HOLY_BLOCKER_SIGNING_IDENTITY="Holy Blocker Dev"
scripts/bundle.sh
.build/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd bundle-status
```

Confirm the last line reads `grants survive a rebuild: true`.

**Recreating after a loss produces a *different* certificate with the same name.** Even though the
subject name matches, the key material is new, so its identity to TCC is new. Any grant made
against the old certificate is gone and Screen Recording / Accessibility must be re-granted through
the real prompt. There is no way around this — it's the same property that makes the certificate
useful in the first place (an attacker can't forge the identity either).

## Rotating it deliberately

Rotate if the private key material is suspected compromised, or as routine hygiene. Same script,
with a flag:

```bash
scripts/create-dev-signing-identity.sh --rotate
```

This deletes the existing `Holy Blocker Dev` identity from the login keychain first, then creates a
fresh one. As with recreation-after-loss, every existing grant is invalidated — re-grant Screen
Recording / Accessibility after the next `scripts/bundle.sh` + agent bootstrap.

## Why it isn't backed up

The private key deliberately lives **only** in the local login keychain:

- It's a development identity scoped to one machine. Nothing depends on it surviving a machine
  loss — recreating it is one script and a few re-clicked TCC prompts.
- Exporting a `.p12` to a shared or synced location (a password manager, cloud drive, the repo)
  would turn a single-machine dev convenience into a credential that needs its own access control
  and rotation policy, for no benefit — a new machine is expected to mint its own.
- It must never be committed to the repository under any circumstance, matching the project's
  local-first, no-cloud-dependency stance in the root `AGENTS.md`.

If a shared team identity is ever needed (multiple developers granting TCC against the same signing
identity so grants are portable across their machines), that's a different, deliberate decision —
likely pointing straight at the Developer ID path below rather than a shared self-signed cert — and
should be made explicitly, not backed into by convenience.

## Before shipping to anyone else

This self-signed identity is **only for exercising TCC locally during development.** Distributing
the app to another person's machine needs an Apple **Developer ID Application** certificate
(requires an Apple Developer Program membership), issued from Apple's own CA so it's trusted
system-wide with no local trust step, and it's the only path that supports notarization — which
Gatekeeper requires before a downloaded, unsigned-by-Apple `.app` will run for anyone but its
builder. That certificate is a distribution decision, tracked separately from this runbook; when it
exists, point `HOLY_BLOCKER_SIGNING_IDENTITY` at its identity name the same way.
