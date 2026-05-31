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

Example decision bands:

```text
score >= 80: block
score >= 50: warn, blur, or log depending on user settings
score < 50: allow
```

The scoring system should support positive and negative evidence:

```text
critical explicit phrase: +100
explicit single term: +60
suggestive phrase: +35
context amplifier: +20
safe educational or medical context: -40
trusted allow phrase: -100
```

## Handling Evasion

Before training a text NLP model, add deterministic evasion handling:

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

