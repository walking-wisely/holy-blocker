# Decision: Protection Modes

## What was decided

Three protection modes: `full`, `warn`, `off`.

| Mode | Scans run | Block verdict | Warn verdict |
|---|---|---|---|
| `full` | yes | blocks / severs | interstitial shown, traffic passes |
| `warn` | yes | interstitial shown, traffic passes | interstitial shown, traffic passes |
| `off` | no | — | — |

Phase 1 (net-shield domain/IP blocklist) is **not** affected by mode. It always runs.
Only the content-analysis layers (text gauntlet, image sandbox, video watchdog, daemon
OCR/image scan) respect the mode.

---

## Why warn passes through instead of blocking

Warn-range scores are ambiguous by definition. The scoring model places content in the
warn band when the evidence is present but not conclusive — it could be a false positive,
an educational context, or genuinely borderline material.

Silently blocking ambiguous content creates two problems:

1. **Frustration from false positives.** A blocked page with no explanation trains the
   user to distrust the system and look for workarounds.
2. **No feedback loop.** If the user never sees what was blocked, they cannot report
   false positives or help calibrate thresholds.

The interstitial solves both: the user sees what was detected, reads the verse, and makes
a deliberate choice. If it was a false positive, they continue and can flag the event in
MonitorView. If it was not, the pause served its purpose.

---

## Why `warn` mode exists (not just `full`)

During initial setup, the configured thresholds may be poorly calibrated for a specific
user's browsing patterns. `warn` mode lets the user run the system in a monitoring-only
state — all traffic passes, all events are recorded — so they can review what would have
been blocked before committing to enforcement.

`warn` mode is also useful during testing and during a transition period after installing
new rule packs.

It is not intended as a permanent operating mode.

---

## Why `off` requires a voice gate but `full`/`warn` do not

The gate is not a security system — it is a friction mechanism. Its purpose is to make
impulsive disabling harder by inserting a deliberate pause (reading a verse aloud) and
creating an accountability signal (notification sent to partner regardless of outcome).

Switching between `full` and `warn` does not disable protection — scans still run in both
modes. Adding gate friction to that switch would create unnecessary friction during
legitimate threshold calibration.

Switching to `off` disables all content scanning (except Phase 1). That is a meaningful
reduction in protection and warrants deliberate pause.

Re-enabling from `off` to `full` or `warn` requires no gate. The gate protects against
lowering protection, not raising it.

---

## Why Phase 1 ignores the mode

Phase 1 (net-shield) operates at the packet level, before TLS decryption and before any
per-request policy decision. It is always-on for two reasons:

1. It is the cheapest layer by orders of magnitude — a radix tree lookup costs
   microseconds, costs nothing in latency for allowed traffic.
2. Known-unholy domains should never reach the proxy regardless of mode. The mode exists
   to soften or suspend the *content-analysis* response, not to allow traffic to known
   bad actors.

A future "emergency bypass" feature (for legitimate domains that appear on the blocklist)
would be handled via an allowlist exception, not by disabling Phase 1 globally.

---

## Alternatives considered

**Two modes only (on / off).** Rejected because it does not support the calibration use
case. Users need a way to observe the system before trusting it enough to enforce.

**Continuous slider instead of three discrete modes.** Rejected because it is hard to
communicate to the user what an intermediate slider value means. Three named modes are
unambiguous.

**`warn` mode shows a notification instead of an interstitial.** Considered but rejected
for the initial implementation. A notification is easy to dismiss without reading. The
interstitial with a verse requires active engagement. Notification-only can be added later
as an optional preference.
