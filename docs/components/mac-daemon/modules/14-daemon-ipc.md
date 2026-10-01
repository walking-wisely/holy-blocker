# 14. `DaemonIPC` — carrying verdicts to the desktop app

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/DaemonIPC.swift
```

The macOS counterpart to the Windows daemon's named-pipe server. Every flow in
[block.md](../../../product/flows/block.md) and
[warn-interstitial.md](../../../product/flows/warn-interstitial.md) ends with the daemon emitting a
`scan_event` that the Electron app records, and
[protection-mode-change.md](../../../product/flows/protection-mode-change.md) sends a `config_update`
back the other way — none of which is possible without this module.

- Transport: a **Unix domain socket** in the user's container, not a TCP port. A localhost port is
  reachable by every process on the machine including a browser page, which for a control channel
  that can set `protection_mode` is a bypass.
- Message shapes match the Windows daemon's so `daemon-ipc.ts` needs one transport adapter rather
  than a second protocol: `scan_event { action, score, source, ts, regions }` outbound,
  `config_update { block_threshold, warn_threshold, protection_mode }` inbound. `regions` is
  `ScanVerdict.regions` (module 9) serialized as normalized rects and is an empty array until a real
  detector or the coarse-grid fallback populates it — see
  [content-classification.md](../../../architecture/content-classification.md#image-localization). Keep
  the field in the shared shape now, even though only macOS produces it today, so the Windows daemon
  does not have to break protocol compatibility to add it later.
- The warn path additionally carries the captured frame for the blurred backdrop. Given the size of
  a Retina frame, encode as JPEG rather than raw BGRA, and hold to the same rule as the overlay: it
  is never written to disk and never logged.
- Framing, parsing and message validation are pure and get tests first; the socket is the edge.

---
