# Voice Gate — Implementation Plan

The voice gate is the override mechanism for the desktop (and future mobile) control panel.
When a user attempts to disable protection, they must read a scripture passage aloud before the
app complies. This document covers the shared package and its platform adapters.

The overall system architecture is described in [../architecture.md](../architecture.md).

## Purpose

Disabling protection should be costly — not impossible, but deliberate. The gate requires:

1. The user speaks a randomly selected scripture verse (transcription must match).
2. A human voice is present (liveness check via anti-spoofing model).
3. The reading is not rushed (speech rate derived from Web Speech API timestamps).

The gate intentionally has no volume floor — users in shared spaces or at night need to be
able to speak quietly. Liveness detection handles the "press play on a recording" bypass
without restricting soft speech.

## Package location

```
packages/voice-gate/
```

A TypeScript package shared by `apps/desktop` (Electron) and the planned React Native mobile
app. It owns all platform-independent logic. Each host app supplies a `VoiceAdapter`
implementation that handles mic access and speech recognition for its platform.

## Architecture

```
VoiceAdapter (platform-supplied)
  ↓  AudioBuffer + Transcript + DurationMs
VoiceGate (this package)
  ├── TranscriptMatcher   — string similarity check
  ├── RateChecker         — words-per-minute from duration
  └── LivenessClassifier  — ONNX anti-spoofing inference
  ↓
GateResult { passed: boolean; failReason?: FailReason }
```

## VoiceAdapter interface

Each platform implements this interface and passes it to `VoiceGate`:

```ts
interface VoiceAdapter {
  startRecording(): Promise<void>
  stopRecording(): Promise<RecordingResult>
}

type RecordingResult = {
  audioBuffer: ArrayBuffer   // raw PCM, 16 kHz mono, 16-bit
  transcript: string         // best-effort transcription from platform SR
  durationMs: number         // wall-clock ms from start to stop
}
```

Platform implementations live outside this package:
- **Electron**: `apps/desktop/src/main/voice-adapter-electron.ts` — uses Node.js `node-record-lpcm16` or the renderer's `MediaRecorder` API bridged via IPC; speech recognition via the renderer's Web Speech API.
- **React Native (future)**: `apps/mobile/src/voice-adapter-rn.ts` — uses `@react-native-voice/voice` and `react-native-audio-record`.

## Modules to add

### `src/verse-picker.ts`

Selects the verse the user must read.

Responsibilities:
- Maintain a curated list of verses relevant to purity, temptation, and perseverance (e.g. 1 Corinthians 10:13, Romans 8:13, James 1:14–15, Psalm 119:9–11, Matthew 5:8).
- `pick(): Verse` — returns a pseudo-random verse not repeated until the full list has cycled.
- Each `Verse` carries `{ reference: string; text: string; wordCount: number }`.

```ts
type Verse = {
  reference: string   // e.g. "1 Corinthians 10:13"
  text: string        // full KJV text
  wordCount: number
}
```

### `src/transcript-matcher.ts`

Checks whether the spoken transcript matches the required verse well enough to pass.

Responsibilities:
- Normalise both strings (lowercase, strip punctuation, collapse whitespace).
- Compute word-level Jaccard similarity between the verse and the transcript.
- Return `{ matched: boolean; similarity: number }`.
- Default pass threshold: 0.80 similarity (configurable).

Leniency is intentional — speech recognition is imperfect, especially for archaic KJV
vocabulary. The check should catch silence or completely wrong speech, not penalise minor
mishearing.

### `src/rate-checker.ts`

Ensures the user is not rushing through the verse.

Responsibilities:
- Accept `{ wordCount: number; durationMs: number }`.
- Compute words-per-minute.
- Reject if WPM exceeds a configurable ceiling (default 160 WPM — comfortable reading pace
  is ~120–140 WPM; 160 gives a small margin before flagging deliberate rushing).
- Return `{ passed: boolean; wpm: number }`.

No ML needed — pure arithmetic from timestamps the speech recognition API already provides.

### `src/liveness-classifier.ts`

Distinguishes a live human voice from silence, TTS playback, or a recording.

Responsibilities:
- Load an ONNX anti-spoofing model (RawNet2 or equivalent) from a path supplied at
  construction time.
- Accept a 16 kHz mono PCM `ArrayBuffer`.
- Run inference via `onnxruntime-node` (Electron) or `onnxruntime-react-native` (mobile) —
  the binding is injected so this module stays platform-neutral.
- Return `{ liveness: number }` where `1.0` = definitely human, `0.0` = synthetic/silent.
- Default pass threshold: 0.6 (configurable).

The ONNX weights live at `packages/voice-gate/models/antispoofing.onnx` (gitignored as a
binary asset; provisioned separately like other models under `data/models/`).

```ts
interface OnnxRuntime {
  run(modelPath: string, input: Float32Array): Promise<Float32Array>
}
```

The `OnnxRuntime` interface is injected by the host so the module itself has no direct
dependency on either ONNX Runtime binding.

### `src/voice-gate.ts`

Top-level orchestrator. Wires verse picker → adapter → matcher + rate checker + liveness
classifier into a single `attempt()` call.

```ts
type GateConfig = {
  transcriptThreshold?: number   // default 0.80
  maxWpm?: number                // default 160
  livenessThreshold?: number     // default 0.60
}

type GateResult =
  | { passed: true }
  | { passed: false; failReason: "transcript" | "rate" | "liveness" | "error" }

class VoiceGate {
  constructor(adapter: VoiceAdapter, classifier: LivenessClassifier, config?: GateConfig)
  currentVerse(): Verse
  attempt(): Promise<GateResult>
}
```

`currentVerse()` is called by the UI before recording starts so the user can see what to read.
`attempt()` starts recording, waits for the adapter to stop (caller signals stop), and
evaluates all three checks. The first failing check short-circuits the result.

## Testing

Tests live in `src/__tests__/`. Use Vitest.

- `transcript-matcher.test.ts` — cover exact match, partial match below threshold, empty
  transcript, punctuation differences, and archaic vocabulary edge cases.
- `rate-checker.test.ts` — cover comfortable pace (pass), rushing (fail), edge cases at
  exactly the WPM ceiling.
- `liveness-classifier.test.ts` — mock the `OnnxRuntime` interface; verify score thresholding
  and that a score below threshold returns the correct `failReason`.
- `voice-gate.test.ts` — integration test using a mock `VoiceAdapter`; verify all three
  pass/fail combinations surface the right `GateResult`.

The ONNX model is not loaded in unit tests — the runtime is always injected as a mock.

## Implementation order

1. `verse-picker.ts` — data only, trivial to test; establishes the `Verse` type used everywhere.
2. `transcript-matcher.ts` — pure function; test-first with the edge cases above.
3. `rate-checker.ts` — pure function; test-first.
4. `liveness-classifier.ts` — inject the ONNX interface; unit test with mocked runtime.
5. `voice-gate.ts` — orchestrator; integration test with mock adapter and mock classifier.
6. `voice-adapter-electron.ts` in `apps/desktop` — wire `MediaRecorder` + Web Speech API;
   manual smoke test in the running app.

## What this does not cover

- Accountability partner notification on override attempt — that is an `apps/desktop` concern
  wired at the IPC layer, not inside this package.
- Selecting between KJV and other translations — the verse list starts KJV; translation
  support can be added to `verse-picker.ts` later without touching the rest of the gate.
- Model training — the anti-spoofing model is a pre-trained public checkpoint. Fine-tuning
  on prayer/reading audio is a future `machine-learning/` task if the pre-trained model
  proves insufficient.
