# Holy Blocker

> "I made a covenant with my eyes not to look lustfully at a young woman."
> — Job 31:1 (NIV)

Holy Blocker is a free, local-first content blocker for people who want to keep that covenant on a modern computer. Most blockers operate only at the domain level — they cannot see what a page *looks* like, cannot block images served from CDNs with clean-looking domains, and cannot catch anything that arrives outside a browser. Holy Blocker intercepts at two independent layers so that what reaches your eyes is what you actually chose to allow.

**Platform:** Windows (Phase 1) · Android planned · macOS / Linux / iOS roadmapped

---

## How it works

Two runtime paths operate independently and reinforce each other:

```
NETWORK PATH (stops content before it renders)
  outbound traffic
    → net-shield     packet filter · SNI/IP radix tree
    → mitm-proxy     TLS termination · HTTP/HTTPS proxy
    → text-policy    metadata + body scan
    → image-sandbox  perceptual hash lookup · ONNX fallback
    → video-watchdog async stream sampler

SCREEN-CAPTURE PATH (backstop for everything else)
  OS event / scan tick
    → win-daemon     foreground hook · pixel capture
    → image model    ONNX on-device classifier
    → OCR            text extraction from screen
    → text-policy    rule + model verdict
    → action         block · blur · warn · log · allow
```

The network path blocks most bad content before the browser renders it. The screen-capture path acts as a backstop for cached pages, native apps, and content that arrives outside the proxy — because the eye does not distinguish a network request from a file on disk, and the blocker should not either.

All decisions are made **on-device**. No screenshots, no OCR text, no browsing content, and no block events are sent to a server.

---

## Accountability

Willpower alone is not the model this project assumes. When a user attempts to disable protection, a designated accountability partner is notified — not only on success, but on the attempt itself. The attempt is meaningful.

Disabling the system requires speaking a scripture passage aloud (voice gate). This is not primarily a security measure. It is a deliberate pause — a moment of engagement with God's word at exactly the moment when the temptation to lower the guard is highest. The friction is the feature.

---

## Project status

| Component | Language | Status |
|---|---|---|
| `apps/desktop` | TypeScript / Electron + React | Skeleton — BrowserWindow, IPC stub, status UI |
| `packages/text-policy` | Rust | Normalize + lexicon + verdict + scorer + evaluator + policy done |
| `packages/mitm-proxy` | Rust | HTTP forwarding + TLS + CONNECT + scan hooks + text-policy wired |
| `native-modules/win-daemon` | C++20 | WinEvent hooks + message loop |
| `machine-learning` | Python | MobileNetV3 model + ONNX export skeleton |
| `packages/net-shield` | Rust | Scaffold + radix domain/IP filter done |
| `packages/image-sandbox` | Rust | Planned |
| `packages/video-watchdog` | Rust | Planned |
| `native-modules/win-network` | C++ | Planned — Wintun driver + Windows routing |

---

## Roadmap

```
NOW   Edge Engines & Local ML Core
      Windows native daemon · pre-trained baseline model · Electron control panel

NEXT  Infrastructure Scale & Platform Parity
      Android AccessibilityService · iOS Network Extension · macOS / Linux daemons
      Federated learning orchestration (on-device weight updates, no user data leaves device)

LATER Absolute Zero-Trust Verification
      Hardware enclave deployment (Intel SGX / AWS Nitro)
      Append-only Merkle tree transparency log — every compiled binary hash is public
      Local network failover — household devices aggregate updates peer-to-peer
```

Full specification: [PLAN.md](PLAN.md)

---

## Getting started (development)

### Prerequisites

- **Node.js** ≥ 20 and **pnpm** ≥ 9
- **Rust** (stable toolchain via `rustup`)
- **CMake** ≥ 3.20 (for the Windows daemon)
- **Python** ≥ 3.10 (for the ML pipeline)
- **Windows 10/11** — the daemon and network modules are Windows-only at this stage

### Install and run

```powershell
# Install JS dependencies
pnpm install

# Start the Electron control panel (hot-reload)
pnpm dev:desktop
```

### Build and test individual packages

```powershell
# Typecheck the desktop app
pnpm --filter @holy-blocker/desktop typecheck

# Build the desktop app
pnpm --filter @holy-blocker/desktop build
```

```bash
# Rust text-policy engine
cd packages/text-policy && cargo test

# Rust MITM proxy
cd packages/mitm-proxy && cargo test

# Rust net-shield
cd packages/net-shield && cargo test
```

```bash
# Python ML pipeline
cd machine-learning && pytest
```

```powershell
# Windows daemon (CMake)
cd native-modules/win-daemon
cmake -B build && cmake --build build
```

---

## Repository layout

```
apps/
  desktop/              Electron control panel

packages/
  text-policy/          Rust text classification and rule engine
  mitm-proxy/           TLS termination, HTTP/HTTPS proxying, scan hooks
  net-shield/           TUN packet reader, SNI extractor, radix-tree domain/IP filter
  image-sandbox/        [planned] Perceptual hashing, SQLite hash lookup, ONNX inference
  video-watchdog/       [planned] Async segment sampler, frame extraction, ML gate

native-modules/
  win-daemon/           Windows foreground hooks and capture loop (C++20)
  win-network/          [planned] Wintun driver binding and Windows routing policy

machine-learning/       Training and export pipeline for local image models

docs/                   Project documentation (architecture, plans, decisions)
```

Each active package has a step-by-step implementation plan in `docs/<package>/PLAN.md`. Read the relevant plan before starting work on a package.

---

## Documentation

- [Foundation](docs/foundation.md) — why this project exists and the theological rationale behind its design
- [Architecture](docs/architecture.md) — the two-path blocking model and component layout
- [Network Pipeline](docs/network-pipeline.md) — the five-phase network interception path
- [Edge Daemons](docs/edge-daemons.md) — Windows and Android daemon strategy
- [Content Classification](docs/content-classification.md) — image, OCR, and text policy decisions
- [Evaluation and CI](docs/evaluation-and-ci.md) — how to test the blocker without sensitive corpora in the public repo

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for branch conventions, commit style, and how to pick up a task.

This is a faith-motivated project. Contributors do not need to share the theological convictions behind it, but should respect that those convictions shape the design decisions and scope. The blocking criterion is not "legally obscene" — it is whether content meets the standard of Philippians 4:8.
