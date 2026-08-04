---
name: holy-blocker-security
description: >
  Review Holy Blocker changes against the project's trust boundaries, and audit existing packages
  for security debt. Use this skill when a change touches network parsing, TLS interception or the
  local CA, Electron preload/IPC, named pipes or Unix sockets, privileged daemons or services,
  OS permission grants, tamper-resistance guards, screen capture or OCR, VPN/TUN logic, or
  dependency and model-artifact loading. Also triggers on "threat model", "security review",
  "is this safe", "audit this package", "attack surface", "can this be bypassed", or any request
  to assess what an attacker could do with a code path.
---

# Holy Blocker Security Skill

This skill runs in one of two modes. Decide which before doing anything else.

**Authoring mode** — a change is being written or reviewed. Scope is the diff. Apply the rules for
whichever boundaries the diff touches, and nothing else. Output is inline feedback.

**Audit mode** — a package is being assessed as a whole. Scope is the package. Output is
backlog entries in `docs/engineering/security-backlog.md`, *not* fixes. Do not fix findings during
an audit; an audit that turns into a refactor stops being an audit.

If the user hasn't said which, infer from scope: a diff means authoring, "audit `packages/x`" means
audit. Ask only if genuinely ambiguous.

## Step 1: route

Read `references/review-triggers.md` and match the touched paths to boundaries. If nothing matches,
say so and stop — a change to `apps/desktop/src/renderer/` styling has no security surface, and
inventing one wastes the reviewer's attention and erodes trust in the gate.

Load only the boundary sections that matched. Most changes touch one.

## Step 2: apply the boundary rules

The router names the boundary; these are the rules per boundary.

### Untrusted input parsers

Any function reading bytes off a socket, a TUN device, or a wire format. In this repo:
`net-shield/src/{dns,udp,sni,tun}.rs`, `mitm-proxy/src/{connect,forward,tunnel}.rs`.

- Every length, offset, and count read from the wire is attacker-controlled. Check it before use.
  Rust panics on slice-out-of-bounds — in a daemon, a panic is a denial of service, not a crash log.
- Reject rather than reconstruct. `dns.rs` refusing compression pointers and `udp.rs` refusing
  fragments are the pattern: a parser that declines an ambiguous input cannot be desynchronised by it.
- Unbounded reads need a cap. A `read_to_end` on a client-controlled stream is a memory exhaustion bug.
- Recursion over nested wire structures needs a depth limit.
- New parsers need the citation comment the project requires (`// RFC 1035 §4.1.4`) and, per the
  test rule, a fuzz or property test — a parser is exactly the shape `proptest` is for.

Ask: what does a malicious server, a malicious page, or a hostile local process on the same machine
send here to make this misbehave?

### TLS interception and the local CA

`mitm-proxy/src/tls.rs`, `mac-daemon/Sources/MacDaemon/CATrust.swift`, and the Windows equivalent.

This is the highest-value target in the repository. A root CA installed in the system trust store
that an attacker can read or abuse defeats TLS for the whole machine.

- The CA private key must never be world-readable, never leave the machine, never be committed, and
  never be a fixed build-time constant shared across installs.
- Generate per-install. A shipped CA key is a universal MITM key for every user.
- Removal must be complete. An uninstall that leaves the root trusted is a permanent downgrade.
- Certificate validation on the *upstream* leg must stay on. Intercepting proxies that skip upstream
  verification turn a MITM defence into a MITM enabler.
- Pinned clients failing closed is correct behaviour, not a bug to work around.

### Privilege boundaries

`mac-daemon` LaunchDaemon (root) vs LaunchAgent (user), `CommandRunner`, Windows service code,
anything that shells out or elevates.

- Anything crossing user → root is an attack surface. Enumerate what the unprivileged side can ask
  the privileged side to do, and confirm the privileged side validates it rather than trusting it.
- Never build a shell command from non-constant input. The project's own rule from
  `docs/decisions/content-interception.md` holds: select among fixed enumerated operations, never
  generate a command. A root helper that takes a string is a confused deputy.
- Check file paths and executable paths the privileged side uses. A root process running a binary
  from a user-writable directory is a privilege escalation.
- Never store a password to automate an admin prompt.

### IPC

Electron preload/`contextBridge`, the Windows named pipe, the mac Unix socket
(`DaemonIPCCodec`), Android Intents and exported components.

- Every IPC message is untrusted input — apply the parser rules above. `DaemonIPCCodec`'s
  `.oversized` case exists for this reason; check any new message type validates before dispatch.
- Named pipes and Unix sockets need an ACL or filesystem permissions that stop another local user
  connecting. Default-permissive is the common mistake.
- Electron: context isolation stays on, node integration stays off, the preload surface stays a
  narrow allowlist of named channels. Never expose `ipcRenderer` itself.
- Android: an exported component with no permission is callable by any installed app.

### Tamper resistance and permission state

Mobile `SettingsGuard`/`ProtectionStore`/`TamperLog`, mac `PermissionGate`, `GuardStatusService`.

- Ask "what is the cheapest silent bypass?" The project's stated goal is no route that is both easy
  and silent — a bypass that is *detected* is acceptable, one that is *invisible* is a finding.
- Watch outcomes, not routes. A check for one specific bypass tool misses the other twenty; a poll
  for the resulting state catches all of them.
- Fail-closed vs fail-open must be deliberate and stated. `image-sandbox` fails open on every path by
  design; a guard that fails open by accident is a hole.
- Anything that can disable protection must be recorded even when it succeeds.

### Content capture and sensitive data

Screen capture, OCR text, AX tree text, URLs, page titles, block events.

- Captured frames and extracted text must not reach disk, a log, a crash report, or an error message.
  Log the decision, never the content — the mobile tamper log's "records the guard, never the screen"
  rule is the standard.
- Check error paths specifically. A URL leaked into an exception string is still a leak.
- Anything leaving the device needs an explicit opt-in that defaults off. This overlaps the privacy
  aspect; flag it and move on rather than adjudicating it here.

### Supply chain and model artifacts

`Cargo.toml`, `package.json`, `pyproject.toml`, Gradle files, and any code loading a model from disk.

- A new runtime dependency with network or filesystem access needs a justification in the PR body.
- `cargo-deny`/`cargo-audit` findings are triaged, not ignored.
- A model file loaded from a writable location is untrusted input to the inference runtime. Check
  where it came from and whether an attacker can replace it — Android `filesDir` provisioning and the
  mac bundle differ here.

## Step 3: report

**Authoring mode** — state the boundary, the concrete attack, and the fix. One finding per real
issue. If the diff is clean, say it's clean; do not pad.

**Audit mode** — append to `docs/engineering/security-backlog.md` using this shape:

```markdown
### [SEV] <package>: <one-line claim>

- **Boundary:** <from the router>
- **Attack:** <who does what, and what they get — concrete, not "could be exploited">
- **Evidence:** `path/to/file.rs:120`
- **Status:** open | accepted-baseline | fixed in <commit>
```

Severity is `[HIGH]` if it is remotely reachable or crosses a privilege boundary, `[MED]` if it needs
local access or user interaction, `[LOW]` if it is defence-in-depth. A finding you cannot write a
concrete attack sentence for is not a finding — drop it.

Recording a gap as `accepted-baseline` is a valid outcome and the point of the audit: it converts
unknown debt into scheduled work. The gate is green on *new* findings, never on a clean package.

## Scope limits

- Ordinary bugs, logic errors, and code quality → `/code-review`, not this skill.
- Test structure and coverage → the `test` skill.
- Workflow file security (pinned actions, permissions blocks) → the `ci-workflows` skill.
- Legal and regulatory questions → out of scope entirely. This skill does not give legal advice.

## Standards this skill draws on

NIST SSDF for lifecycle structure, OWASP ASVS where app/API requirements apply, Electron's official
security guidance, OWASP MASVS/MASTG for Android, Microsoft SDL and the Win32 IPC docs for the
Windows service, Apple Platform Security for macOS, SEI CERT C++ for native C++, and
RustSec/cargo-deny/SLSA for supply chain.

Deliberately **not** targets: ISO 27001, SOC 2, PCI DSS, HIPAA, NIST 800-53, Common Criteria, and
MISRA C++. These are organisational governance or embedded-safety frameworks, not coding guidance
for this repository — see `docs/engineering/aspect-skills-plan.md` for the reasoning.
