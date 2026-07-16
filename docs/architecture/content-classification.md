# Content Classification

Holy Blocker should combine image classification, OCR, and text policy rather than depending on only one technique.

```text
captured pixels
  -> image model
  -> OCR provider
  -> text policy engine
  -> final decision
```

## OCR Strategy

OCR is an extraction step, not the moderation decision itself. The OCR provider should return text plus confidence and location metadata where available.

Start with a provider interface:

```text
OcrProvider.recognize(image, language_hint) -> OcrResult
```

Windows-native OCR is useful for the first Windows build because it is local and integrated with the OS. It should not be the only long-term OCR path if broad language support is required. The project should support fallback providers such as Tesseract or a future ONNX/PaddleOCR-based provider.

## Text Policy Engine

The text policy engine should be shared across apps and daemons. A native implementation is preferred over TypeScript for portability and speed.

Recommended implementation language: Rust.

Reasons:

- safer Unicode-heavy string handling than C++;
- strong regex, tokenization, and automata libraries;
- practical native builds for Windows, macOS, Linux, Android, and iOS;
- can expose C ABI, JNI, UniFFI, N-API, or WASM bindings;
- easy to test as an isolated package.

Proposed future package:

```text
packages/text-policy/
  Cargo.toml
  src/
    lib.rs
    normalize.rs
    tokenize.rs
    rules.rs
    score.rs
  rules/
    en.json
    uk.json
    ru.json
```

## Decision Pipeline

The first implementation should be deterministic and explainable:

```text
OCR text
  -> Unicode normalization
  -> case folding
  -> script/language hinting
  -> tokenization
  -> exact word and phrase matching
  -> evasion normalization
  -> fuzzy or regex-based matching
  -> context scoring
  -> decision threshold
```

The pipeline should produce a trace of intermediate evidence, not only a final
boolean result. This makes false positives debuggable and keeps the future NLP
model from becoming the only source of truth.

Recommended flow:

```text
raw text from OCR, browser title, URL, or accessibility tree
  -> input cleanup
  -> Unicode normalization
  -> case folding
  -> script and language hinting
  -> base tokenization
  -> normalized token variants
  -> suspicious window generation
  -> deterministic rule matching
  -> context and exception checks
  -> score aggregation
  -> decision band
  -> optional NLP model for uncertain cases
```

### Normalization Workflow

Normalization should preserve the original text for audit/debug output while
creating safer matching forms for the policy engine. The engine should never
replace the original evidence with only the normalized text.

1. Input cleanup

   Remove surrounding whitespace, normalize line endings, collapse repeated
   whitespace, and discard control characters that do not affect visible text.
   This keeps OCR artifacts from creating artificial token boundaries.

2. Unicode normalization

   Convert text to a stable Unicode form such as NFKC where appropriate. This
   folds visually similar compatibility characters, full-width letters, and
   presentation forms into a more predictable representation. Keep this step
   conservative because some scripts use marks and combined characters in valid
   ways.

3. Case folding

   Use Unicode-aware case folding rather than ASCII-only lowercasing. This makes
   matching consistent across languages and avoids missing uppercase or mixed
   case forms.

4. Script and language hinting

   Detect likely scripts and language hints from OCR metadata, app settings, and
   character ranges. This should route text to the right language pack, not
   pretend to perfectly identify language. Mixed-script text should be marked as
   suspicious when it appears inside a sensitive token.

5. Base tokenization

   Split the cleaned text into tokens, separators, and positions. Positions
   matter because the engine needs to explain which part of the screen matched
   and because phrase matching depends on token distance.

6. Repeated character normalization

   Create a variant where suspicious character runs are reduced, for example
   `heyyy` -> `hey`. Do not blindly collapse all repeats to one character,
   because some languages and ordinary words use repeated letters legitimately.
   Prefer rules such as "reduce runs longer than two" and apply them mostly
   inside suspicious windows.

7. Separator stripping for suspicious windows

   Generate compact variants for short token windows where letters are separated
   by punctuation, spaces, or symbols. This catches evasion patterns like a word
   split across dots or hyphens without stripping separators across a whole
   paragraph.

8. Leetspeak mapping

   Create variants using small, language-aware maps such as `0 -> o`,
   `1 -> i/l`, `3 -> e`, `4 -> a`, `5 -> s`, `7 -> t`, and common symbol
   substitutions. Apply this as a candidate variant, not as the only text form,
   because many numbers are legitimate.

9. Homoglyph normalization

   Map visually confusable characters only when the script context makes it
   likely to be evasion, for example a mostly Latin token containing one Cyrillic
   lookalike. This should be stricter than normal case folding because broad
   homoglyph replacement can damage valid multilingual text.

10. OCR-confusion normalization

   Add variants for common OCR mistakes such as `0/O`, `1/l/I`, `rn/m`, and
   broken punctuation. Keep OCR-specific variants tied to OCR confidence and
   source type. Text typed directly by a browser or accessibility API should not
   need the same level of OCR repair.

11. Language-specific token rules

   Apply only the language packs that were hinted or configured. Start with
   curated keyword variants and phrase variants. Add stemming or lemmatization
   only when tests show that manual variants are not enough for that language.

12. Candidate generation limits

   Cap the number of variants per token/window. Normalization can otherwise
   explode into many possible strings, making the engine slow and increasing
   false positives.

Each normalization stage should produce structured candidates:

```text
original span
normalized text
normalization type
confidence
language/script hint
source position
```

The matcher then scores candidates with their provenance. A direct exact match
on visible text should count more than a weak match that only appears after
multiple speculative normalizations.

### Rule Matching

Rules should live in external policy bundles rather than being hard-coded in
the public repository. Public tests can use synthetic terms.

Useful rule types:

- exact token: a whole normalized token must match;
- exact phrase: adjacent normalized tokens must match in order;
- window phrase: phrase terms may be separated by a small number of tokens;
- compacted phrase: a suspicious window matches after separator stripping;
- regex/fuzzy rule: limited to carefully reviewed cases because it can overmatch;
- allow phrase: trusted context that reduces or cancels a match;
- context amplifier: surrounding words that make a weak match more likely to be
  policy-relevant.

Rules should return evidence, not immediate decisions:

```text
rule id
category
severity
matched span
match type
normalization path
base score
confidence multiplier
```

### Scoring Workflow

Scoring should combine positive evidence, negative evidence, source confidence,
and user settings.

1. Assign a base score by rule severity

```text
critical explicit phrase: +100
explicit phrase: +80
explicit single term: +60
suggestive phrase: +35
weak contextual signal: +15
```

2. Adjust by match quality

```text
exact visible phrase: x1.00
url host/path token match: x1.00
case-folded match: x0.95
repeated-character variant: x0.85
separator-stripped variant: x0.80
leetspeak variant: x0.75
homoglyph variant: x0.70
OCR-confusion variant: x0.60 to x0.85, depending on OCR confidence
fuzzy/regex-only match: x0.50 to x0.75
```

A URL match carries full confidence despite resembling a separator-stripped
match. A term in a host or path has no surrounding prose that could make it
innocent, so there is nothing for a discount to hedge against — and because a
phrase is never contiguous across URL separators, it can never match as an
exact visible phrase. Discounting it would put the block band permanently out
of reach for URLs.

**Score each occurrence once, at its best match quality.** A single occurrence
generally matches in several ways at once — the same word can satisfy the exact,
separator-stripped, and leetspeak variants simultaneously — and these are one
occurrence seen through different normalization views, not several hits. Take
the highest quality among them and score it once. Summing across variants would
let the number of variants a rule author happens to enable drive the score
instead of the rule's severity, which is how a suggestive term (+35) can reach
a band meant for explicit phrases (+80).

Repeated *occurrences* of a term do accumulate. Occurrences are counted within a
single variant, since spans from different normalization views are not
comparable to each other.

3. Adjust by source confidence

OCR text should be weighted by OCR confidence and location quality. Text from a
browser title, URL, or accessibility tree can usually be treated as higher
confidence than OCR.

```text
browser/accessibility text: x1.00
high-confidence OCR: x0.90
medium-confidence OCR: x0.70
low-confidence OCR: x0.40
```

4. Apply context amplifiers

Nearby evidence can raise the score when several weak signals point in the same
direction.

```text
same category repeated in a short window: +15
phrase plus matching image category: +20
user-configured high-risk category: +20
known risky app/site context: +10
```

5. Apply safe-context reductions

Trusted educational, medical, pastoral, or moderation contexts can reduce the
score. These should be conservative and traceable.

```text
trusted allow phrase: -100
educational or dictionary context: -40
medical or support context: -40
quoted policy/documentation context: -30
user allowlisted source: -50 to -100
```

6. Clamp and explain

Clamp the final score to a fixed range such as `0..100`. Return the top evidence
items and the main score modifiers so the user or maintainer can understand why
the decision happened.

Example decision bands:

```text
score >= 80: block
score >= 50: warn, blur, or log depending on user settings
score < 50: allow
```

## Handling Evasion

Before training a text NLP model, add deterministic evasion handling in the
text policy engine:

- repeated character normalization;
- separator stripping for suspicious token windows;
- leetspeak maps;
- homoglyph normalization;
- OCR-confusion normalization;
- phrase-window scoring;
- language-specific token rules.

## When To Add ML

Do not train a custom NLP classifier before the project has a useful eval set and enough real false positive/false negative examples.

Add a small multilingual text model later for cases where rules are weak:

```text
rules score high: block
rules score low: allow
rules uncertain: run text model
```

This keeps the hot path fast, inspectable, and easy to tune.

The NLP model should receive the original text, selected normalized forms,
language/script hints, deterministic rule evidence, and source confidence. It
should not replace deterministic rules. Its first job is to help with ambiguous
cases, such as whether a term is being discussed educationally, quoted in a
moderation context, or used as an attack.

Recommended gating:

```text
score >= block threshold: block without NLP unless user settings require review
score <= allow threshold: allow without NLP
middle band: run NLP model
model agrees with rules: apply stronger decision
model disagrees with rules: warn, blur, or ask for review depending on settings
```

Model outputs should be treated as another evidence source:

```text
model category probabilities
model confidence
detected language
short explanation code, not free-form moral judgment
```

The final decision should still be made by the policy engine so user settings,
allowlists, accountability settings, and pastoral/educational exceptions remain
outside the model.

## Crowdsourced Translations

Crowdsourced rule contributions can help language coverage, but they must be treated as untrusted inputs. Store metadata with each term or phrase:

```text
language
script
category
severity
review status
source notes
safe-context exceptions
```

Sensitive full rule packs should not live in the public repository unless intentionally sanitized.
