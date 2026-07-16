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
| [formation-model.md](formation-model.md) | What the tool may judge vs. what only the person may judge; two thresholds (block on recall, invite on precision); strictness controls sensitivity not certainty; show shape never sum; no streaks; release deletes the episode and keeps the pattern; why the reflection surface stays small |
| [crisis-surface.md](crisis-surface.md) | **Open — deferred.** Records unanswered clinical/pastoral questions rather than decisions. Gates all reflection beyond the one-tap self-report. Reopens whether partner mode should exist |
| [content-interception.md](content-interception.md) | Cross-platform content interception: two-layer model (network proxy + capture/ML render path), per-platform instantiation (Windows/Linux/macOS/Android/iOS), why injection is deferred, tamper resistance |
