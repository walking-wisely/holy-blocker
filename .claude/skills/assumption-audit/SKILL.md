---
name: assumption-audit
description: >
  Verify a module's claims about the outside world before writing code for it. Use this skill
  before implementing any step of a `docs/components/<package>/plan.md`, before accepting a plan
  an agent wrote, and whenever a design rests on how an OS API, a wire protocol, a resolver, a
  model, or a third-party app actually behaves. Triggers on "implement the next step",
  "start module N", "is this plan right", "what does this assume", "before we build this", or any
  request to build against an API or protocol whose real behaviour has not been observed in this
  repo yet.
---

# Assumption Audit

Run this **before** implementation, not after. It exists because of a measured pattern in this
repository: every module here rests on one to three claims about the outside world, and when those
claims go unverified they are wrong often enough to invalidate the module — not its details, its
purpose.

The evidence is in `CLAUDE.md`'s own status rows and in `references/falsification-recipes.md`.
Six-plus instances across four packages and three languages. In every case the falsifying command
was cheap and available before line one was written.

The single most expensive one so far: `packages/domain-blocklist`'s liveness module was built,
reviewed, and fixed across ~21 commits before an adversarial review found that `Verdict::Dead` is
unreachable for `.com`/`.net`/`.org` because those zones use NSEC3 opt-out and never set `AD`.
Three `dig` commands would have found it on day zero.

## What this skill is not

It is not a design review, a security review (`holy-blocker-security`), or a code review
(`adversarial-review`). It asks exactly one question, about the world rather than the code:

> Which facts outside this repository does this module need to be true, and are they?

## Step 1: enumerate the external claims

Read the plan section for the module about to be built. Extract every statement that could be
falsified by something outside this repository. Sources of claims, in the order they hide best:

- **Defaults.** Any API whose behaviour depends on a value nobody set. Defaults are the top source
  of wrong claims in this repo — see the `SCStreamConfiguration.pixelFormat` case.
- **Units.** Points vs pixels, bytes vs rows, seconds vs milliseconds, score 0–1 vs 0–100.
- **Protocol behaviour in the wild**, as distinct from protocol behaviour in the RFC. The RFC says
  what is permitted; deployments say what happens. Both matter and they differ.
- **Platform grant, signing, and identity semantics.** Who is the responsible process, what keys a
  grant, what invalidates it.
- **Third-party application behaviour.** What Chrome, Firefox, Electron, or an OEM Android skin
  actually does, versus what its documentation or a blog post says.
- **Numeric constants inherited from elsewhere.** A threshold belongs to a model *and* a geometry;
  a size floor belongs to a measurement. A constant with no measurement behind it is a claim.
- **Availability.** That a crate, runtime, model artifact, or prebuilt binary exists for the target
  triple. `ort` has no prebuilt runtime for two of Android's three ABIs; that cost the mobile image
  path its whole design.

Write each claim as a sentence that can be false. "The stream delivers BGRA frames" is a claim.
"We use ScreenCaptureKit" is not.

Aim for the three to seven claims the module would collapse without. A list of twenty means the
false ones are being padded with the trivially true.

## Step 2: attach a falsifier to each claim

For each claim, name **one command** whose output settles it, and say what result would falsify the
claim. Read `references/falsification-recipes.md` for the recipes this repo has already needed —
DNS, macOS TCC and signing, Android permissions and services, ONNX and model artifacts, browsers.

Rules that make this cheap rather than a research project:

- Prefer a command that runs in under a minute on the development machine.
- Prefer observing the real thing over reading about it. A blog post is not a falsifier. Apple's
  own documentation is not a falsifier for TCC behaviour — `strings` over `tccd` is
  (`CLAUDE.md` records the case where the documented key list did not exist in the shipped binary).
- If the only honest falsifier needs hardware or a grant that is not available, say so and mark the
  claim **unverifiable here**. That is a legitimate outcome. It is *not* the same as verified, and
  the plan must carry the distinction — the Recents/One UI item in the mobile backlog is the
  correct precedent.
- If a claim can only be settled by reading source, read the *shipped* artifact, not upstream.
  Firefox's `omni.ja` is a plain zip and is versions behind mozilla-central.

## Step 3: run them

Run every falsifier. Record the actual output, trimmed to the line that matters — not a summary of
it. A summary of an unverified command is how a fabricated citation gets in.

Do not run the falsifiers selectively. The one that looks most obviously true is the one worth
running: `INTERNET` being present, the frame being BGRA, and `.com` being signed all looked
obviously true.

## Step 4: report before writing code

Append a table to the module's section in `docs/components/<package>/plan.md`:

```markdown
### Assumption audit

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| `.com` NXDOMAIN carries `AD=1` | `dig +dnssec nonexistent.com @1.1.1.1 \| grep flags` | `flags: qr rd ra` — no `ad` | **FALSE** — NSEC3 opt-out (RFC 5155 §6) |
| … | … | … | … |
```

Then **stop and report to the user before implementing.** State plainly:

- which claims held;
- which were false, and what that does to the module — if a false claim removes the module's
  purpose, say so in those words and propose the redesign rather than building around it;
- which are unverifiable here and what would be needed to settle them.

A module whose load-bearing claim is false does not get built with a note in the plan. That is the
failure mode this skill exists to stop: `CLAUDE.md` contains several findings phrased as
"contradicts the plan it was written from", each recorded honestly and each after the code existed.

## Step 5: keep the audit next to the code

Constants that came out of a falsifier carry the measurement in a comment at the definition, in the
same style the project requires for specification citations:

```rust
/// Measured 2026-08-09 against 1.1.1.1/8.8.8.8/9.9.9.9: only .se, .nl, .cz, .app, .dev, .top
/// return AD=1 for authenticated denial. Do not raise this without re-measuring.
const REQUIRE_AUTHENTICATED_DENIAL: bool = false;
```

An unannotated constant is a claim that has lost its evidence, which is how both of
`image-sandbox`'s v0 constants came to be wrong at once.
