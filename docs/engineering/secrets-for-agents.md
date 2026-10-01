# Secrets for agent-driven end-to-end runs

Status: **proposal, nothing implemented.** Today's end-to-end paths (the Android smoke tests,
the Rust crates, the desktop dev server) need no credentials at all; the project is local-first
and has no cloud calls. This page exists so the first secret that does appear lands in the
right place rather than in a `.env`.

## What will need one

| Need | Secret | Who consumes it |
|---|---|---|
| Signed blocklist production | Blocklist signing key | `net-shield` signing tooling |
| Release builds | Android keystore + passwords | `./gradlew assembleRelease` |
| Model provisioning | Hub/registry token, if a model host requires one | ML export tooling |
| PRs and CI inspection | GitHub auth | `gh` (already handled by `gh auth`, not by this scheme) |

## Why not `.env`

This repo runs dozens of parallel worktrees. A gitignored `.env` is copied or symlinked into each
one, so it:

- drifts — a rotated value is updated in one worktree and stale in the rest;
- mixes two different things: **secrets**, which are identical everywhere, and **instance
  config** (ports, device serials, temp dirs), which must differ per worktree;
- outlives the worktree reaper, which deliberately skips ignored state;
- is readable by the agent with one `cat`, which puts the value in a transcript.

## Proposal

**1. Separate the two kinds of value.**
Instance config is never stored: it is derived at run time (Portless assigns ports,
`ANDROID_SERIAL` is set per emulator). Only secrets are stored, and only once, globally.

**2. Store secrets in the OS keychain, not the filesystem.**
One item per secret, service name `holy-blocker.<NAME>` (`security add-generic-password` on
macOS). File-shaped secrets such as a keystore live in a per-user directory outside every
worktree, and the keychain item holds the path and passphrase.

**3. Commit a manifest of names, never values.**
`secrets.toml` at the repo root maps a *profile* to the secrets it needs and the variable each
becomes — for example `release = ["ANDROID_KEYSTORE_PATH", "ANDROID_KEYSTORE_PASSWORD"]`.
CI uses the same names as GitHub Actions secrets, so a profile means the same thing locally
and in CI.

**4. Inject at exec time through one wrapper.**
`scripts/with-secrets <profile> -- <command…>` reads the manifest, fetches only that profile's
values, and `exec`s the command with them in its environment. Values never touch disk, are not
exported into the parent shell, and die with the process, so nothing leaks between worktrees or
into later commands. The wrapper masks the values in anything it prints.

**5. Draw the agent boundary at the wrapper.**
The agent is allowed to run `scripts/with-secrets <profile> -- …` for named profiles and is
denied direct keychain reads (`security find-generic-password -w`) and reads of the per-user
secrets directory, through permission rules. It never sees a value. Profiles with an outward
effect (release signing, publishing) additionally require a confirmation each time; profiles
that only read, or sign local test artifacts, do not.

## Rejected

- **direnv / `.envrc`**: exports into the interactive shell, which every agent command then
  inherits; also per-directory, so it re-creates the worktree-drift problem.
- **A committed encrypted file (sops/age)**: workable, but the decryption key still has to
  live somewhere, and rotation becomes a commit.

## Open decision

Keychain (zero dependencies, macOS only) versus 1Password `op run` (cross-platform, needs the
CLI and a signed-in session). The wrapper is the same either way, about forty lines; only its
fetch step differs. Windows work (`win-network`) will need the second.
