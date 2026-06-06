# Decision: Verse Selection

## What was decided

Verses are shown in two contexts:

1. **Warn interstitial** — triggered when a content scan returns a Warn verdict. Shorter
   verses; the user reads silently and makes a quick deliberate choice.
2. **Override gate** — triggered when the user attempts to disable protection. Longer,
   more reflective passages; the user reads aloud as part of the gate.

Each context has its own verse pool. Verses are static, embedded in the binary/bundle.
No network call is made at display time.

---

## Category-to-pool mapping (warn interstitial)

The warn interstitial selects from the pool whose category best matches the dominant
evidence item from the policy engine verdict.

| Evidence category | Verse theme |
|---|---|
| `ExplicitAct` | purity of mind, fleeing lust |
| `Nudity` | the body as a temple, modesty |
| `AdultPlatform` / `CommercialAdult` | guarding your eyes, what you dwell on |
| `MedicalException` | not shown — exception context reduces score below warn threshold |
| fallback (no dominant category) | guarding the heart / mind generally |

If multiple categories appear in the evidence, the category with the highest accumulated
score in the verdict is used as the dominant category.

Pool size target: 5–10 verses per category for v1. Enough variety to avoid repetition
across a single session without requiring a large content database.

---

## Selection within a pool

- A verse is sampled randomly from the relevant pool.
- The last shown verse id is tracked in memory (not persisted across sessions).
- The same verse is not repeated consecutively within a session.
- No global history is kept — repetition across sessions is acceptable.

---

## Why verses instead of generic warnings

A generic "this content may be harmful" banner is cognitively easy to dismiss. The user
has seen it before. It requires no engagement.

A verse requires the user to actually read and process meaning. The pause is the point.
It creates a brief reflective moment rather than a mechanical click-through, which is
especially important for the override gate where the verse is read aloud.

The verse is not shown as a condemnation. The overlay tone should be calm and inviting,
not accusatory. The user is given a choice, not a verdict about themselves.

---

## What verses are not

- The verse pool is not a public blocklist. Verse text is pastoral content, not moderation
  evidence. It lives in the app bundle and is not secret.
- Verses are not generated, summarised, or fetched from any external service. The pool is
  curated and static.
- The verse shown does not identify what specific content was detected. It responds to the
  category of concern (purity of mind, guarding the eyes), not to any particular URL or
  keyword found on the page.

---

## Storage format

A single embedded JSON file per context, keyed by category:

```json
{
  "ExplicitAct": [
    { "id": "1co6-18", "reference": "1 Corinthians 6:18", "text": "Flee from sexual immorality…" },
    { "id": "mt5-28",  "reference": "Matthew 5:28",        "text": "…" }
  ],
  "Nudity": [ … ],
  "AdultPlatform": [ … ],
  "fallback": [ … ]
}
```

The file lives at:
- Proxy (Rust): compiled in via `include_str!` or `serde_json` from `data/verses/warn.json`
- Desktop (TS): imported as a static JSON module bundled by Vite

Both use the same source file so the pool is maintained in one place.

---

## Alternatives considered

**Show the matched text that triggered the detection.** Rejected — displaying the matched
content defeats the purpose and could be jarring or re-exposing. The verse should redirect
attention, not highlight the offending text.

**Dynamic verse selection based on page content (ML or keyword matching).** Rejected for
v1. The category-based mapping is sufficient and avoids the complexity of a second
classification pass just for verse selection.

**User-configurable verse pool.** Possible future feature. For v1, the pool is curated
by the project maintainers. User-added verses can be added later via a settings panel.
