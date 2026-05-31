# Windows Daemon

Skeleton Win32 background process for the Phase 1 event-driven scanner.

Current responsibilities:

- Subscribe to foreground window changes with `SetWinEventHook`.
- Subscribe to window move/resize mutations.
- Keep a low-cost message loop alive for system events.

Planned responsibilities:

- Capture candidate window regions.
- Run OCR/image classification through ONNX Runtime.
- Emit local-only events to the desktop app over a named pipe or loopback IPC.
