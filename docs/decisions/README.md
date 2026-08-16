# Design decisions

Each file captures one significant design choice: what was decided, why, and what was
rejected. Read these when you need to understand the reasoning behind a constraint before
changing it.

| Decision | Summary |
|---|---|
| [protection-modes.md](protection-modes.md) | Why three modes (Full / Warn / Off), why warn passes through, why Off requires a gate |
| [verse-selection.md](verse-selection.md) | Category-to-verse mapping, pool format, why verses instead of generic warnings |
| [verse-pools.md](verse-pools.md) | Curated NIV verse text for the warn interstitial and override gate pools |
| [accountability.md](accountability.md) | Partner notifications, why the notification fires on attempt, counts-only weekly summary |
| [learning-from-feedback.md](learning-from-feedback.md) | Why text scoring is demoted, domains as the floor, local personalization, and the federated design for false-positive reduction under a temptation-biased labeler |
| [classifier-operating-point.md](classifier-operating-point.md) | Why false negatives are the budget and false positives the price, why accuracy is not the headline metric, and how the deployed threshold is derived |
| [content-interception.md](content-interception.md) | Cross-platform content interception: two-layer model (network proxy + capture/ML render path), per-platform instantiation (Windows/Linux/macOS/Android/iOS), why injection is deferred, tamper resistance |
| [domain-blocklist-sourcing.md](domain-blocklist-sourcing.md) | The CSAM legal boundary, the three upstream sources, the merge/provenance/liveness pipeline, and the mmap'd FST on-device format |
| [image-corpus-custody.md](image-corpus-custody.md) | Why no third-party imagery is ingested at any scale, which photo sources are permitted, the retention surface to hold, and what becomes permanently unmeasurable as a result |
