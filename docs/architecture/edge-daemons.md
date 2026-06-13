# Edge Daemons

Edge daemons should combine OS events with a foreground scan loop. Events provide fast wakeups and state hints. The scan loop provides correctness for dynamic content that does not reliably emit accessibility or window events.

## Windows Daemon

The Windows daemon currently uses `SetWinEventHook`. The first useful implementation should track the foreground window and scan it on an interval.

Recommended baseline:

```text
Event thread:
  - track current foreground HWND
  - track window bounds and minimized/desktop state
  - signal immediate scan after major state changes

Scanner thread:
  - every 500 ms, inspect current foreground HWND
  - skip invalid, minimized, hidden, or unchanged surfaces where safe
  - capture foreground window pixels
  - run image classifier
  - run OCR/text policy at a controlled cadence
  - apply local action
```

## Minimal Windows Event Set

If the scanner runs every 500 ms, events do not need to cover every possible content mutation. They only need to make major surface changes visible immediately.

Recommended minimum:

```cpp
EVENT_SYSTEM_FOREGROUND
EVENT_OBJECT_LOCATIONCHANGE
EVENT_SYSTEM_MINIMIZESTART
EVENT_SYSTEM_MINIMIZEEND
EVENT_SYSTEM_DESKTOPSWITCH
```

This can be expanded later if the daemon maintains a full window map or if profiling shows too much wasted polling:

```cpp
EVENT_OBJECT_SHOW
EVENT_OBJECT_HIDE
EVENT_OBJECT_CREATE
EVENT_OBJECT_DESTROY
EVENT_OBJECT_CLOAKED
EVENT_OBJECT_UNCLOAKED
EVENT_OBJECT_CONTENTSCROLLED
EVENT_SYSTEM_SCROLLINGEND
EVENT_SYSTEM_MOVESIZEEND
```

## Scan Cadence

A 500 ms foreground scan is reasonable for a first protective loop, but OCR should not necessarily run every tick.

Recommended cadence:

```text
Image classifier:
  - every 500 ms while the foreground surface is eligible

OCR:
  - every 1000-2000 ms
  - immediately after foreground changes
  - immediately after meaningful visual frame difference

Text policy:
  - whenever OCR returns new or meaningfully changed text
```

Use frame hashing or perceptual difference checks to avoid rerunning expensive OCR on identical frames.

## Android Daemon

The Android service should use accessibility events as high-value scan signals:

```text
TYPE_WINDOW_STATE_CHANGED
TYPE_VIEW_SCROLLED
TYPE_WINDOW_CONTENT_CHANGED
```

As on Windows, these events should not be the only correctness mechanism. Dynamic content and custom-rendered views may still require periodic or diff-triggered scans while a monitored surface is active.

