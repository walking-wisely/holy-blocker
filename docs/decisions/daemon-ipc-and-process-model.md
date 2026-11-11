# ADR: One install, several processes, a root-owned policy daemon and a shared IPC core

| Field | Value |
|---|---|
| **Status** | Accepted 2026-11-11 on the owner's answers |
| **Date** | 2026-11-11 |
| **Owner** | Ivan Dutov |
| **Supersedes** | [tauri-desktop-panel.md](tauri-desktop-panel.md) decisions 3 and 5 as to where policy state and the socket live; [custom-app-blocking.md](custom-app-blocking.md) decision 16's wait for `mac-daemon.standard-user-tamper`; [mvp-scope.md](mvp-scope.md) owner answers 4 and 5 as to signing and updates |
| **Superseded by** | — |

Tier: architecture decision that moves a trust boundary and sets a contract two or more platforms
consume. Red-team tier 2, three lenses, under
[decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md); findings and their
dispositions are at the end.

---

## Context

The owner wants one sleek install per platform, with several background processes: one that blocks
and one that shows the UI. Three things surfaced while building `desktop.daemon-socket`:

- The wire framing exists twice and the plans already disagree: Swift and Rust use a 4-byte
  big-endian length ([DaemonIPC.swift:207-223](../../native-modules/mac-daemon/Sources/MacDaemon/DaemonIPC.swift)),
  the Windows plan says little-endian ([win-daemon plan](../components/win-daemon/plan.md) line 190).
  The Rust test vectors are read by no Swift test.
- A same-user process can flood the socket (probed on a throwaway listener: a 256-fd process stopped
  accepting at 251 connections, then failed to open a file) and can flush a 2,000-entry tamper-log
  ring (static reasoning).
- The list, the cooldown clock and the tamper log live in the user-level agent, so a standard user
  can edit them. [custom-app-blocking.md](custom-app-blocking.md) decision 16 accepted that and held
  any change for the owner after `mac-daemon.standard-user-tamper` had measured it.

## Decision

1. **One signed bundle, several processes.** The bundle carries the panel (Tauri, user), the agent
   (Swift, user: capture, AX, overlay) and a root daemon (proxy, CA, policy state). All are built
   from one commit and versioned together.

2. **The root daemon owns policy state** (owner, accepted before the standard-user tamper
   measurement; the red-team's objection is recorded below). The app list, the monotonic cooldown
   clock, the tamper log, the block counts and the panel socket move from the agent to a root
   LaunchDaemon. The agent and the panel are clients. The socket sits in its own root-owned directory, mode 0755 with the socket at 0666 gated by the peer check of decision 4, because a user-level client cannot pass through the 0700 store directory; the exact path is fixed by `mac-daemon.root-policy-store`. This amends
   tauri-desktop-panel decisions 3 and 5 and sets the install layout, which is a one-way door.
   - **What it protects, stated plainly:** a standard user can no longer edit the list, reset the
     cooldown, rewrite the log or squat the endpoint. **It does not protect the blocking:** capture,
     AX and app termination stay in the user-level agent, which a standard user can still stop,
     SIGSTOP or strip of its permissions. The daemon detects that (decision 9) and keeps the
     proxy-level blocking running; text and app blocking pause until the agent returns.
   - **Store posture, carried from custom-app-blocking decision 13:** directory
     `/Library/Application Support/holy-blocker/`, root-owned, mode 0700, files 0600, excluded from
     Time Machine, never in iCloud. The store is readable by an admin and by no standard account; that
     scope is stated wherever the posture is claimed. Retention is bounded per tier (decision 6).
     Uninstall removes the store and the log. A `data-classes.md` row is added in the same change.
   - **Logging:** the daemon's stdout and stderr go to `/dev/null`; no frame body, app identity or
     host name is formatted into an error, a `tracing` event, a panic message or a crash report.
     Message types carrying an app identity do not derive `Debug`.

3. **A shared sans-IO Rust crate, `packages/daemon-ipc`, built fully** (owner). It holds framing, the
   message schema, the version handshake, the session state machine, the limits, the rate limiter,
   liveness, the redaction rules and the shared test vectors. It does no I/O: the transport (Unix
   socket, named pipe) and the peer checks stay with each platform, and policy (cooldown, tamper log)
   stays with the server.
   - API: `Session::new(role, limits, now_ms)`, `hello()`, `receive(bytes, now_ms) -> Vec<Event>`,
     `tick(now_ms) -> Option<CloseReason>`, encode functions. Time is an injected monotonic `u64`.
   - Messages are two serde enums, `ToDaemon` and `ToPanel`, tagged `type` in snake_case. The Rust
     types are the schema; JSON vectors written by hand from this decision are the golden files.
   - The panel depends on the crate directly. Swift gets a `daemon-ipc-ffi` UniFFI wrapper whose every
     export returns `Result`, built with `panic = unwind`; no Swift call site uses `try!`. A
     later `daemon-ipc.c-abi` step adds cbindgen for Windows when `win-daemon.ipc` starts.
   - The length prefix is big-endian. The Windows plan changes to match.

4. **Roles and peer authentication.** The root daemon accepts two client roles, `panel` and `agent`,
   each with its own connection slot; unauthenticated connections never occupy a slot. The transport
   reads the peer's audit token (`LOCAL_PEERTOKEN`, never the pid) and checks the code signature
   against a requirement pinned to the Developer ID team and the bundle identifier of that role. Agent
   messages are accepted only from the signed agent, panel messages only from the signed panel. A
   panel connecting to the daemon checks the daemon's signature the same way.

5. **Versioning.** The first frame each way is `{"type":"hello","proto":N}`; N must lie in
   `MIN_PROTO..=PROTO`, and any other message before `hello` closes the connection. Changes are
   additive. Raising `MIN_PROTO` removes old shapes, and a message that lowers protection is removed
   in the release that introduces its replacement, not phased out. An unknown `type` from a newer
   peer is reported as `Unsupported` and the connection stays open; a malformed or oversized frame
   closes it. On a mismatch the panel shows "updating" and retries, and after N retries the daemon
   records a tamper event.

6. **Limits and the tamper log.** Inbound frame cap 4 KiB toward the daemon and 64 KiB toward the
   panel; 5 s to finish a frame; 60 s idle; close on the first invalid frame; a per-connection token
   bucket of 10 messages per second and 1 state-changing message per second, with over-limit messages
   dropped and counted. `cancel_disarm` and `raise_protection` are exempt, because dropping a
   protective action fails open. The file descriptor limit is raised at start and the log fd opened
   once. A repeated request while one is pending is a no-op and writes nothing. The tamper log is
   severity-tiered: low-severity entries (requests, cancels, throttles) are coalesced into one counted
   entry per window, held in memory, and carry a count and a window, never a peer, path or message
   text; they can never evict `agent_stopped`, `disarm_confirmed`, a version decrease or a permission
   revocation. Every tier has a stated size bound, set in the step that builds it.

7. **Cooldown clocks survive reboot and clock changes.** Every monotonic stamp is stored with the boot
   session id (`kern.bootsessionuuid`, read-only, probed). A mismatch voids the stamp and purges it,
   and voiding returns to the most protective state. At each start the daemon checks whether the
   previous boot was a safe mode and records it. This also fixes a suspected reboot-window bug in the
   mobile schedule (static reasoning; its own step reproduces it first).

8. **Distribution: Developer ID, notarization and `SMAppService`** (owner). The bundle is signed with
   the Developer ID Application identity (present in the build host's keychain, probed), notarized
   with the hardened runtime, and installed in `/Applications`. The helpers are registered through
   `SMAppService` (`agent(plistName:)` for the agent, `daemon(plistName:)` for the root daemon;
   probed to exist, status `notFound` when unregistered), and the daemon starts only after an admin
   approves it in System Settings. This amends mvp-scope owner answers 4 and 5. Consequences:
   - a release workflow with notarization credentials in CI secrets (the repo has none today); the
     secrets are the owner's to provide and are never read by an agent;
   - a development build falls back to the legacy plists in `/Library/LaunchDaemons` through the
     dispatcher ([agent-privilege-boundary.md](agent-privilege-boundary.md)), because a self-signed
     build cannot be notarized;
   - Apple sees a hash of the notarized binary; this is an owner-visible fact, not a data flow of the
     person's content;
   - no updater in the panel or the daemon. The daemon refuses a `CFBundleVersion` not above the
     installed one and records a version decrease or a missing app as tamper. That check protects
     against a casual downgrade only: an admin removes it.

9. **Liveness by heartbeat, not by the connection closing.** This is the daemon's internal check on the agent, not the user-facing liveness signal that mvp-scope decision 2 keeps out of the MVP. A stopped (SIGSTOP) agent keeps its
   socket open, so the daemon expects a heartbeat from the agent and records `agent_stopped` when it
   stops. When the agent is stopped the daemon keeps blocking at the proxy and records the tamper
   event, as the existing Layer 1 plan does; this is unchanged behaviour restated, not a new owner
   decision.

10. **Windows is not in the first cut.** The Windows plan is amended to big-endian, to drop the raw
    `config_update` that contradicts tauri-desktop-panel decision 3, and to consume `daemon-ipc` when
    `win-daemon.ipc` starts. win-network's newline-delimited framing is out of scope. A default
    Windows user is an administrator, so Windows tamper resistance needs its own owner decision.

11. **Out of this decision, by name:** shipping evidence off the device (which tamper fields may
    leave, partner opt-in and who holds the signing key are decided first, per
    [accountability.md](accountability.md) and [mission.md](../mission.md)), a Network Extension
    filter for VMs and browsers that ignore the system proxy (Apple entitlement, its own inception),
    any updater, and the identity of the installing admin when someone other than the owner installs
    the product. The release build is for the owner's own devices; the consent rule for installing it
    on another person's machine is decided before anyone else installs it.

## Claims, tagged

| Claim | Domain | Status |
|---|---|---|
| The owner wants one install with several background processes | product | PASS: owner's words, this session |
| Root-owned policy state, the crate built fully, Developer ID with `SMAppService` | product | PASS: owner's answers, 2026-11-11 |
| A standard user cannot edit root-owned state | engineering | UNVERIFIED: observe on a standard test account; the dev user is in `admin` (probed). `mac-daemon.standard-user-tamper` now measures the result of this decision |
| `SMAppService` agent and daemon APIs exist on the build host | engineering | PASS: probed on macOS 27.2 |
| A bundled daemon needs admin approval and notarization | engineering | PASS for the SDK header; end to end UNVERIFIED |
| A Developer ID identity is present | engineering | PASS: probed; notarytool credentials UNVERIFIED |
| A standard user cannot unregister or disable an approved daemon | engineering | UNVERIFIED: observe on a standard test account |
| UniFFI wraps a stateful streaming decoder with no callbacks | engineering | PASS: probed, Swift typechecks |
| A panic crosses UniFFI as a thrown error under `panic = unwind` | engineering | UNVERIFIED: the first `daemon-ipc.ffi` test settles it |
| `LOCAL_PEERTOKEN` is available and a code-signature requirement can be checked from it | engineering | PASS for the SDK symbol (probed); the signature check UNVERIFIED |
| `kern.bootsessionuuid` is read-only | engineering | PASS: probed |
| A SIGSTOP'd process keeps its socket open | engineering | PASS: probed |
| Windows framing is little-endian with no cap | engineering | PASS (static): win-daemon plan line 190 |
| Flooding exhausts file descriptors | engineering | PASS on a throwaway listener; the daemon's real limit under launchd UNVERIFIED |
| The tamper log can be flushed by request/cancel churn | engineering | UNVERIFIED (static reasoning) |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| Wait for `mac-daemon.standard-user-tamper` before moving state | The owner chose to move now; that step now measures the result instead of gating the choice |
| Keep two codecs and share only vectors | Already drifted (endianness); the limits would be written twice |
| A schema file generating both sides | A second source of truth beside the Rust types |
| Transport inside the shared crate | Unix socket and named pipe differ; ACLs and peer checks belong to the platform |
| A daemon-driven updater with a pinned key | A root process installing without an admin present is the strongest bypass of the cooldown |
| Sparkle or the Tauri updater in the panel | Contradicts tauri-desktop-panel decision 7 and the local-first rule |
| Homebrew cask as the primary channel | A one-line `uninstall --zap` removal and a third-party channel |
| Reinstall-only `.pkg` with the self-signed identity | The owner chose Developer ID now |
| A C ABI for Windows now | No Windows IPC code exists; the step follows `win-daemon.ipc` |
| Connection-close as the liveness signal | A SIGSTOP'd agent keeps its socket open (probed) |

## Red-team findings and what became of them

Three fresh-context reviews, read-only, 2026-11-11: bypass, intent, legal and privacy. Probes are
marked; the rest is static reasoning.

| # | Lens | Finding | Disposition |
|---|---|---|---|
| 1 | Bypass | The daemon never authenticated its clients, so a same-uid script could pose as the agent | Decision 4 |
| 2 | Bypass | A shared two-client cap lets one rogue connection lock out the panel or the agent | Decisions 4 and 6 (own slot per role, unauthenticated connections hold none) |
| 3 | Bypass | Root state does not protect blocking; SIGSTOP and TCC revocation remain | Decisions 2 and 9 |
| 4 | Bypass | `MIN_PROTO` downgrade and an endless "updating" state | Decision 5 |
| 5 | Bypass | A decoder panic kills the root daemon that also hosts the proxy | Decision 3 (`Result` exports, `panic = unwind`, no `try!`), fuzzing in `daemon-ipc.core` |
| 6 | Bypass | Dropping `cancel_disarm` or `raise_protection` fails open | Decision 6 (exempt) |
| 7 | Bypass | Boot-session voiding direction and safe mode | Decision 7 |
| 8 | Bypass | The version floor lives in the binary it guards | Decision 8 (stated as casual-downgrade only) |
| 9 | Intent | Moving state precedes the measurement decision 16 asked for, and is framed as a recommendation | Escalated; the owner accepted. The step now measures the result |
| 10 | Intent | Adds 5-8 steps ahead of the single demo with the shared crate unconsumed by Windows | Escalated; the owner chose the full crate. The sequence is in the plan |
| 11 | Intent | Version range and rate limits are more than one bundle needs | Escalated with 10; kept at the owner's request |
| 12 | Intent | The partner question is outside the MVP's solo mode | Decision 11 (deferred) |
| 13 | Privacy | The root store had no path, mode, backup posture or retention | Decision 2, store posture; `data-classes.md` row |
| 14 | Privacy | Flood counters and entries could carry a peer, a timing or an identity | Decision 6 (memory only, count and window) |
| 15 | Privacy | A root process, its logs and crash reports are unreviewed | Decision 2, logging |
| 16 | Privacy | Pkg receipts, notarization and uninstall leave traces or leftovers | Decisions 2 and 8 (uninstall removes the store; Apple sees a hash) |
| 17 | Privacy | Off-device evidence needs decisions first | Decision 11 |
| 18 | Privacy | A root monitoring-capable daemon on a shared machine needs a consent rule | Decision 11 |

## What this does not cover

- An admin, or anyone who can build the public repo, can install, downgrade or remove the product
  ([mvp-scope.md](mvp-scope.md)); signing and notarization raise the cost of casual tampering only.
- A standard user can still stop the capture agent. The daemon detects it and keeps the proxy
  blocking; text and app blocking pause. Whether a standard user can also disable the approved
  daemon through Login Items is **unverified**.
- Live patching and reverse engineering of the binaries is not defensible by local software.
- Safe mode, booting another volume and a VM are detectable at best.
- A bug in the shared parser is identical on both ends; hand-written vectors and property tests are
  the check, not a round trip between the two ends.
- Legal exposure of a persistent root helper is **unverified**: no statute or Apple rule on
  monitoring software was checked, and none is asserted.
