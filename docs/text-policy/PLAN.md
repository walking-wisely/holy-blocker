# Text Policy Engine — Implementation Plan

The design rationale and scoring model live in [content-classification.md](../content-classification.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

The package already has:

- `normalize` — multi-step, language-aware normalization pipeline producing multiple text views (normalized, leet-compact, separator-tokens, raw).
- `lexicon` — dictionary-based term matching across all four match modes (ExactPhrase, TokenSequence, Compact, UrlTokenSequence). Produces `LexiconMatch` results carrying term id, category, severity, match mode, surface, and span. Internally split into sub-modules: `types`, `builder`, `automaton`, `matcher`, and `url` (URL token-sequence matching).

What is missing is everything above the lexicon: aggregating matches into a score, mapping the score to a verdict, and exposing the result to callers.

## Modules to add

### 1. `verdict` — action types

```
src/verdict.rs
```

The single output type that consumers care about:

```rust
pub enum Action { Block, Blur, Warn, Log, Allow }

pub struct Verdict {
    pub action: Action,
    pub score:  u32,       // 0..=100
    pub evidence: Vec<EvidenceItem>,
}

pub struct EvidenceItem {
    pub rule_id:   String,
    pub category:  Category,
    pub severity:  Severity,
    pub span:      MatchSpan,
    pub base_score: u32,
    pub multiplier: f32,
}
```

No logic here — just the output shapes. The design in `content-classification.md` (§ Rule Matching) lists the full evidence fields; start with the subset above and extend when the scorer needs them.

### 2. `scorer` — match → score

```
src/scorer.rs
```

Takes a slice of `LexiconMatch` and a `SourceKind` (see below) and returns a `u32` score in `0..=100`.

Responsibilities:

- Assign a base score per severity/category following the bands in `content-classification.md` (§ Scoring Workflow).
- Apply the match-quality multiplier based on `MatchMode` (exact phrase gets ×1.00, compact/leet gets ×0.75, etc.).
- Apply the source-confidence multiplier based on `SourceKind`.
- Apply safe-context reductions for exception categories (`MedicalException`, `EducationException`, `SafetyException`).
- Clamp to `0..=100`.

`SourceKind` encodes where the text came from:

```rust
pub enum SourceKind {
    BrowserTitle,
    BrowserUrl,
    AccessibilityTree,
    OcrHigh,
    OcrMedium,
    OcrLow,
}
```

The scorer should be a pure function — no I/O, no state. It receives evidence and returns a number.

### 3. `evaluator` — score → verdict

```
src/evaluator.rs
```

Applies the decision bands from `content-classification.md` against user-configurable thresholds:

```rust
pub struct Thresholds {
    pub block: u32,  // default 80
    pub warn:  u32,  // default 50
}

pub fn evaluate(score: u32, evidence: Vec<EvidenceItem>, thresholds: &Thresholds) -> Verdict
```

The action assignment:

```
score >= thresholds.block  →  Block
score >= thresholds.warn   →  Warn (Blur or Log depending on caller settings)
otherwise                  →  Allow
```

For now `Blur` and `Log` are caller-selected variants of the warn band; the evaluator does not need to distinguish them.

### 4. `policy` — top-level entry point

```
src/policy.rs
```

Wires normalize → lexicon → scorer → evaluator into a single call:

```rust
pub struct PolicyEngine {
    matcher:    LexiconMatcher,
    thresholds: Thresholds,
}

impl PolicyEngine {
    pub fn new(matcher: LexiconMatcher, thresholds: Thresholds) -> Self
    pub fn evaluate(&self, text: &str, source: SourceKind) -> Verdict
    pub fn evaluate_normalized(&self, views: &NormalizedText, source: SourceKind) -> Verdict
}
```

`evaluate_normalized` is the hot path for callers (proxy, daemon) that pre-normalize for performance or want to reuse views across multiple checks.

### 5. ML hook (deferred)

`content-classification.md` (§ When To Add ML) describes gating ML on the uncertain middle band. Do not implement this until there is a real eval set. Leave a clear hook point in `evaluator`:

- When `score` falls in the uncertain band (`warn <= score < block`), the evaluator can optionally call an `MlClassifier` trait before making the final decision.
- The trait should take evidence and the original `NormalizedText` and return a probability and a label.
- Wire it to `None` for now; the scorer path is correct without it.

## Implementation order

1. `verdict.rs` — types only, no logic; tests are trivial.
2. `scorer.rs` — pure function; test with synthetic matches and known expected scores.
3. `evaluator.rs` — pure function; test all three bands and edge cases at thresholds.
4. `policy.rs` — integration; test end-to-end from raw text to `Verdict`.
5. FFI surface — once `PolicyEngine` is stable, add a thin `#[no_mangle]` wrapper or UniFFI descriptor so the daemon and proxy can call it. Keep FFI in a separate `src/ffi.rs` or a sibling crate.

## What this does not cover

- Rule bundle format (JSON/TOML packs for `rules/en.json` etc.) — the lexicon already handles dictionary loading; the bundle format is a serialization concern for a later iteration.
- Crowdsourced term contributions — handled separately per `content-classification.md` (§ Crowdsourced Translations).
- OCR-confidence normalization and homoglyph normalization — the normalization pipeline can grow these as additional steps; the scorer multipliers above already have slots for them.
