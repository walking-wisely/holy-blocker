# Flow: Protection Mode Change

**Trigger:** user selects a new protection mode in the desktop app (PolicyView segmented
control or tray menu) or an automated routine resets the mode on a schedule.

Three modes exist: `full`, `warn`, `off`. See
[../decisions/protection-modes.md](../../decisions/protection-modes.md) for why.

---

## Gate check

```
requested mode == "off"
  → override gate must pass first (see override-gate.md)
  → if gate fails: mode is not changed, flow ends here

requested mode == "full" or "warn"
  → no gate required; proceed immediately
```

Lowering protection requires friction. Raising it never does.

---

## Steps

### 1. Renderer → main process

```
user action in PolicyView or tray menu
  → window.holyBlocker.setProtectionMode(mode)
  → IPC: protection:set-mode
```

### 2. Main process validation and persistence

```
ipc-handlers.ts: protection:set-mode handler
  → validate: mode ∈ { "full", "warn", "off" }
  → if mode == "off": run override gate (override:attempt); abort on failure
  → read userData/policy.json (or defaults)
  → write { ...existing, protectionMode: mode } to policy.json atomically
    (write to .tmp sibling, then rename)
  → emit protection:mode-changed on main-process event bus
```

### 3. Propagate to daemon

```
daemon-ipc.ts receives protection:mode-changed
  → send config_update message over named pipe:
    { block_threshold, warn_threshold, protection_mode: "full"|"warn"|"off" }
  → daemon ScanLoop updates its atomic<ProtectionMode> field
  → takes effect on next Tick() call (no restart required)
```

### 4. Propagate to proxy

```
proxy main.rs holds Arc<AtomicU8> for ProtectionMode
  → on config_update received from desktop IPC or reloaded from policy.json:
    mode_atom.store(new_mode, Ordering::Relaxed)
  → takes effect on next HTTP request (no restart required)
```

*(The channel from desktop → proxy is not yet implemented. Interim: proxy reads
policy.json at startup only. Full live propagation is deferred until the proxy runs
as a supervised subprocess of the desktop app.)*

### 5. UI update

```
renderer re-reads getProtectionMode() after successful write
  → PolicyView segmented control reflects new mode
  → threshold inputs grey out if mode == "off"
  → tray icon colour updates:
      "off"  → grey
      "warn" → blue (if no recent events) or amber/red (if recent events)
      "full" → green / amber / red depending on recent events
  → tray menu label updates: "Full protection ✓" / "Warn only ✓" / "Off"
```

### 6. Accountability notification

```
if mode == "off" and accountability partner email is configured:
  → accountability:notify-override-attempt fires regardless of gate outcome
  → email sent: timestamp, device name, outcome ("protection disabled")
```

---

## Effect on running scans

| Component | "full" | "warn" | "off" |
|---|---|---|---|
| net-shield Phase 1 | blocks known-unholy | blocks known-unholy | blocks known-unholy |
| proxy Phase 3–5 | block + warn flows active | block downgraded to warn overlay | scanning skipped, all pass |
| daemon scan loop | full verdict emitted | Block downgraded to Warn in ScanVerdict | Tick() skips IScanner::Scan |

Phase 1 (net-shield domain/IP blocklist) is not affected by ProtectionMode. It is a
separate, always-on filter. Only the content-analysis layers (text, image, video) respect
the mode.

---

## Related

- [override-gate.md](override-gate.md) — the voice gate triggered when switching to "off"
- [block.md](block.md) — what happens in "full" mode when score ≥ block threshold
- [warn-interstitial.md](warn-interstitial.md) — what happens in "warn" mode
- [../decisions/protection-modes.md](../../decisions/protection-modes.md) — design rationale
