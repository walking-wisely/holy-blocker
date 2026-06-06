# User-facing flows

Each file describes one complete scenario end-to-end — trigger, what each layer does,
and what the user sees. Start here when implementing a cross-cutting feature.

| Flow | Trigger | Layers involved |
|---|---|---|
| [block.md](block.md) | Score ≥ block threshold, mode is `full` | net-shield, proxy, daemon, desktop |
| [warn-interstitial.md](warn-interstitial.md) | Score in warn band, mode is `full` or `warn` | proxy (HTML injection now), daemon (native overlay future), desktop |
| [protection-mode-change.md](protection-mode-change.md) | User changes mode in UI or tray | desktop → daemon → proxy |
| [override-gate.md](override-gate.md) | User attempts to set mode to `off` | desktop, voice-gate package |
| [partner-setup.md](partner-setup.md) | User adds an accountability partner | desktop, invite web page, email |

For design rationale behind these flows see [../decisions/](../decisions/).
For system topology see [../architecture.md](../architecture.md).
