# Design decisions

Each file captures one significant design choice: what was decided, why, and what was
rejected. Read these when you need to understand the reasoning behind a constraint before
changing it. New decisions are authored by the `plan-inception` skill, which writes the
decision record to this index **before** drafting any `plan.md`; no plan should cite a
decision that is not recorded here.

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
| [external-content-classifiers.md](external-content-classifiers.md) | Why cloud/System-One classifiers (Jev) are rejected, why Laya/VisionLaya are deferred, and why image-quality work goes into local established classifiers instead |
| [decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md) | Product, architecture and implementation decision tiers, which reach the owner, and the tiered red-team (none, one lens, three lenses) run before a decision is recorded |
| [feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md) | A demo is an e2e scenario with an interactive tail; local not CI regression, preflight vs failure, what cannot be automated, and the macOS and Android gotchas |
| [e2e-scenario-contract.md](e2e-scenario-contract.md) | The scenario manifest, exit codes, the runner-decided pass, a shared pass record in git notes keyed on the covered-path tree hash, where staleness is surfaced, and owner-confirmed promotion to Covered |
| [agent-privilege-boundary.md](agent-privilege-boundary.md) | How the agent runs privileged steps on the dev machine: a root-owned dispatcher of enumerated verbs behind one sudoers rule, never a stored password or a root shell |
