# ADR: Agent privilege boundary on the development machine

| Field | Value |
|---|---|
| **Status** | Accepted — design only; nothing is installed |
| **Date** | 2026-10-01 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

Local e2e scenarios ([feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md)) need
privileged operations: installing and removing the dev CA in the System keychain, applying and
restoring proxy settings, bootstrapping the root `LaunchDaemon`, resetting a TCC entry. The
owner does not want to sit at the keyboard typing a password, and does not want the agent in
a root shell.

The agent reads web pages and files, so it can be steered by prompt injection. Whatever it is
allowed to do as root must be harmless when called by a misled agent.

## Decision

The security boundary is a **fixed list of operations**, not a transport.

1. **A root-owned dispatcher plus one sudoers drop-in.** The drop-in grants `NOPASSWD` for
   exactly one absolute path, a root-owned script in a directory the user cannot write
   (e.g. `/usr/local/libexec/holy-blocker/dispatch`). The agent calls
   `sudo -n <dispatcher> <verb> [args]`. With `-n`, a call that is not permitted fails
   instead of prompting.
2. **Verbs are enumerated with fixed or validated arguments.** Candidate verbs: install and
   remove the dev CA; apply and restore proxy settings; bootstrap and bootout the Holy Blocker
   launchd jobs; reset a TCC entry for the Holy Blocker bundle ID only; revoke the drop-in.
   There is no generic "run this command" verb and no wildcard in the sudoers rule. This is
   the rule `content-interception.md` already records for a privileged helper: select among
   fixed operations, never generate a shell command.
3. **Root never executes a user-writable file.** If a verb runs a binary from the repo
   checkout, the agent could swap it and become root. The dispatcher copies the file to a
   root-owned location and verifies its code signature against the `Holy Blocker Dev` identity
   before running it, then runs the copy.
4. **Every verb is reversible and scoped to this project's artifacts.** A misled call must not
   be able to damage anything outside them. A verb that cannot meet that is not added.
5. **Every call is logged** with verb, arguments, and caller, to a root-owned file.
6. **An MCP server is optional and sits on top.** If built, it runs as the user, never as root,
   and only wraps the verbs with structured output. Running it as root would make any tool that
   accepts free-form arguments a root shell for the agent.
7. **Installation is manual and one-time.** The owner reviews the files and installs them
   (`! sudo ...`). The agent writes the files and never installs them. A `revoke` verb removes
   the drop-in.

## Verified here

- `visudo -cf` accepts the single-path `NOPASSWD` form (`parsed OK`).
- `sudo -n true` on this machine fails with `a password is required` without the drop-in, so
  `-n` does fail closed.
- The development account is in the `admin` group.

## Not verified

- That a sudoers rule naming one script path resists argument smuggling in practice. The man
  page's warning about wildcards and writable targets is the basis; the design avoids both,
  but a `holy-blocker-security` review of the dispatcher is required before it is installed.
- Behaviour across a macOS upgrade; `/etc/sudoers.d` handling is not guaranteed to be
  preserved by the OS.

## Limits

- **Dev machine only.** The development account is in the `admin` group, which is the
  unprotected configuration for the product's own tamper model. This setup must never ship and
  must not be mistaken for the product's privileged helper.
- **It cannot grant TCC.** `TCC.db` is SIP-protected and root is not sufficient
  (`content-interception.md`). The manual grants in the demo ADR's §5 stay manual.
- **A GUI session is still needed** for anything that prompts in the window server. Screen
  Sharing supplies one for a remote machine; SSH does not.

## Rejected

- **A stored password, or a long-lived root shell for the agent.** One prompt injection away
  from arbitrary root.
- **`NOPASSWD: ALL`, or a sudoers rule with wildcard arguments.** Equivalent to a root shell;
  the repo's own analysis of GTFOBins-style escapes applies.
- **A root MCP server taking free-form tool arguments.** Same failure, with more surface.
- **Putting the privileged helper in the product.** The product's helper is a separate design
  governed by `content-interception.md`.

## What this does not cover

- Windows and Linux. Elevation there (UAC, `sudo` inside a UTM VM) is a different mechanism
  and is deferred until those platform contracts exist.
- The dispatcher's implementation. See `docs/engineering/plan.md` step 11.
