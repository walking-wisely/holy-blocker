# Windows Daemon — Implementation Plan

The intended daemon responsibilities and scan cadence are defined in [edge-daemons.md](../../architecture/edge-daemons.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Related flows

- [../flows/block.md](../../product/flows/block.md) — daemon scan_event on Block verdict
- [../flows/warn-interstitial.md](../../product/flows/warn-interstitial.md) — future full-screen native overlay on Warn verdict
- [../flows/protection-mode-change.md](../../product/flows/protection-mode-change.md) — how config_update updates ScanLoop mode at runtime

## Current state

The package at `native-modules/win-daemon/` already has:

- `main.cpp` — registers two `SetWinEventHook` handles (`EVENT_SYSTEM_FOREGROUND` and `EVENT_OBJECT_LOCATIONCHANGE`), extracts the foreground window title and bounds in the callback, logs them to stdout, and runs a standard Win32 message loop to keep the process alive.
- `CMakeLists.txt` — minimal single-source build: one executable target, C++20, MSVC `/W4 /permissive-` flags, no external dependencies.

What is missing is everything above event observation: capturing pixels from the foreground window, routing frames through a scanner interface, debouncing and scheduling scans, and communicating results to the Electron control panel over IPC.

## Modules to add

### 1. `capture` — GDI window capture

```
src/capture.h
src/capture.cpp
```

Exposes a single entry point and the frame type it produces:

```cpp
struct CapturedFrame {
    std::vector<uint8_t> pixels;  // raw BGRA, row-major
    int width;
    int height;
    FILETIME timestamp;
};

CapturedFrame CaptureWindow(HWND hwnd);
```

Responsibilities:

- Use `PrintWindow` (preferred — works for off-screen and DWM-composited windows) or `BitBlt` against the window DC as a fallback.
- Allocate and fill a DIB section; copy pixel data into the returned `CapturedFrame`.
- Return an empty frame (zero width/height) when the window is invalid, minimized, or capture fails; callers must check.
- All GDI handle lifetimes (`HDC`, `HBITMAP`) are scoped to this translation unit. No raw handles escape.

Platform API isolation makes this unit-testable by passing a pre-built bitmap path or a fake HWND backed by an off-screen surface in tests.

### 2. `scanner` — scan interface and null stub

```
src/scanner.h
```

Defines the abstract interface that the scan loop drives, plus the initial no-op implementation:

```cpp
enum class ScanAction { Allow, Warn, Block };
enum class ScanSource { OCR, Image, Text };

struct ScanVerdict {
    ScanAction action;       // effective action after applying ProtectionMode
    float      score;        // 0.0–1.0
    ScanSource source;
    ScanAction raw_action;   // action as returned by IScanner, before mode downgrade
};

class IScanner {
public:
    virtual ~IScanner() = default;
    virtual ScanVerdict Scan(const CapturedFrame& frame) = 0;
};

// Initial stub — always returns Allow; replaced once a real classifier exists.
class NullScanner : public IScanner {
public:
    ScanVerdict Scan(const CapturedFrame& frame) override;
};
```

Responsibilities:

- Keeps ONNX Runtime and OCR calls behind this interface so the scan loop does not depend on any inference library yet.
- `NullScanner` is the only concrete implementation at this stage; it returns `{Allow, 0.0f, ScanSource::Image}` for every frame.
- Real classifiers (`OnnxScanner`, `OcrScanner`) are added later without touching the scan loop.

### 3. `scan_loop` — debounce and state machine

```
src/scan_loop.h
src/scan_loop.cpp
```

The scan loop is the scheduler that bridges WinEvent callbacks and the scanner interface:

```cpp
enum class ProtectionMode { Full, WarnOnly, Off };

struct ScanLoopConfig {
    std::chrono::milliseconds debounce_ms{200};
    std::chrono::milliseconds scan_interval_ms{500};
    std::atomic<ProtectionMode> protection_mode{ProtectionMode::Full};
};

class ScanLoop {
public:
    explicit ScanLoop(IScanner& scanner, ScanLoopConfig config = {});

    // Called from the WinEvent callback thread.
    void OnForegroundChange(HWND hwnd);
    void OnLocationChange(HWND hwnd);

    // Called from the scanner thread (or a timer) on each tick.
    // Returns the most recent verdict, or nullopt if skipped.
    std::optional<ScanVerdict> Tick(FILETIME now);
};
```

Responsibilities:

- Debounce: discard events that arrive within `debounce_ms` of the previous event; only the last event in a burst produces a capture.
- Maintain a simple state machine: `Idle → Capturing → Scanning → Acting → Idle`. Transitions are driven by `Tick`.
- On a scan trigger: call `CaptureWindow` for the current foreground HWND, pass the frame to `IScanner::Scan`, return the verdict.
- Skip capture when the HWND is null, the window is minimized, or the same surface was scanned within `scan_interval_ms` with no intervening events.
- The state machine and debounce logic are pure with respect to time: `Tick` accepts `FILETIME now` so tests can inject a fake clock.
- Wire into `main.cpp` by constructing a `ScanLoop` in `main` and forwarding `HandleWinEvent` events to it.

### 4. `ipc` — named pipe server

```
src/ipc.h
src/ipc.cpp
```

Manages the named pipe channel between the daemon and the Electron control panel:

```cpp
// Pipe name: \\.\pipe\holy-blocker-daemon

enum class MessageType { Heartbeat, ScanEvent, StatusUpdate, ConfigUpdate };

struct IpcMessage {
    MessageType type;
    std::string json_payload;
};

class IpcServer {
public:
    IpcServer();
    ~IpcServer();

    // Non-blocking; connects one pending client if available.
    void AcceptPending();

    // Broadcast a message to all connected clients.
    void Send(const IpcMessage& msg);

    // Drain inbound messages from all clients.
    std::vector<IpcMessage> Receive();
};
```

Outbound message payloads:

| `type`          | Key fields                                              |
| --------------- | ------------------------------------------------------- |
| `heartbeat`     | `{"ts": "<iso8601>", "version": "<semver>"}`            |
| `scan_event`    | `{"action": "block\|warn\|allow", "score": 0.0, "source": "ocr\|image\|text", "ts": "<iso8601>"}` |
| `status_update` | `{"state": "idle\|scanning\|acting"}`                   |

Inbound `config_update` payload:
```json
{
  "block_threshold": 0.8,
  "warn_threshold": 0.5,
  "protection_mode": "full"
}
```
`protection_mode` ∈ `{ "full", "warn", "off" }`. See [../flows/protection-mode-change.md](../../product/flows/protection-mode-change.md)
for the full propagation flow and [../decisions/protection-modes.md](../../decisions/protection-modes.md)
for rationale. The mode is stored in `ScanLoopConfig::protection_mode` as a
`std::atomic<ProtectionMode>` and updated on receipt without restarting the loop.

Responsibilities:

- Wrap the Win32 named pipe API (`CreateNamedPipe`, `ConnectNamedPipe`, `ReadFile`, `WriteFile`) behind the `IpcServer` interface; no pipe handles escape the implementation file.
- Frame messages with a 4-byte little-endian length prefix followed by UTF-8 JSON.
- Handle broken pipe errors gracefully: drop the disconnected client and continue serving others.
- `IpcServer::Send` and `Receive` are callable from any thread; use a mutex to protect the client list.
- Test send/receive in isolation with a loopback client that opens the same pipe name.

### 5. `CMakeLists.txt` updates

Add the new sources to the main executable target and introduce a test target:

```cmake
# Main executable — add new sources alongside main.cpp
add_executable(holy-blocker-win-daemon
  src/main.cpp
  src/capture.cpp
  src/scan_loop.cpp
  src/ipc.cpp
)

# GoogleTest via FetchContent
include(FetchContent)
FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG        v1.14.0
)
FetchContent_MakeAvailable(googletest)

add_executable(holy-blocker-win-daemon-tests
  tests/capture_test.cpp
  tests/scan_loop_test.cpp
  tests/ipc_test.cpp
)
target_link_libraries(holy-blocker-win-daemon-tests PRIVATE GTest::gtest_main)
include(GoogleTest)
gtest_discover_tests(holy-blocker-win-daemon-tests)
```

Logic-only source files (`scan_loop.cpp`, `ipc.cpp`) should be compiled into a static library shared between the main executable and the test target to avoid compiling them twice once the project grows.

## Implementation order

1. `capture.h/cpp` — pure GDI capture; test with a synthetic off-screen bitmap to confirm pixel layout and empty-frame behavior on a null HWND.
2. `scanner.h` — interface plus `NullScanner` stub; no ONNX yet; verify that `NullScanner::Scan` compiles and returns `Allow`.
3. `scan_loop.h/cpp` — debounce and state machine driven by `NullScanner`; unit-test all state transitions and debounce behavior with a fake clock and a fake scanner that records calls.
4. `ipc.h/cpp` — named pipe server; test send/receive round-trip with a loopback client in the same test process.
5. Wire scan loop into `main.cpp`: forward `HandleWinEvent` events to `ScanLoop::OnForegroundChange` / `OnLocationChange`, run `Tick` on a background thread timer.
6. Wire IPC heartbeats: emit a `heartbeat` message from the background thread every 5 seconds so the Electron app can detect daemon presence.
7. Update `CMakeLists.txt` with `FetchContent` GoogleTest and the `tests` target.

## What this does not cover

- ONNX Runtime integration and real image classification — deferred until a trained model artifact exists in `data/models/image-v1/`.
- Windows native OCR (`Windows.Media.Ocr`) integration — deferred; requires WinRT headers and async/coroutine plumbing.
- Accessibility tree scanning — planned; see [edge-daemons.md](../../architecture/edge-daemons.md).
- Expanded WinEvent set (`EVENT_SYSTEM_MINIMIZESTART`, `EVENT_SYSTEM_DESKTOPSWITCH`, etc.) — the current two hooks are sufficient for the scan loop; the full recommended set from [edge-daemons.md](../../architecture/edge-daemons.md) can be added alongside the accessibility work.
- Android daemon — lives in `apps/mobile/`; not covered here.
- Frame-difference hashing to skip redundant OCR — noted in [edge-daemons.md](../../architecture/edge-daemons.md); depends on OCR being wired first.
