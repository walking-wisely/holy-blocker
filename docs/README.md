# Holy Blocker Documentation

This directory contains the project documentation for Holy Blocker. The files are plain Markdown by design so they can be rendered directly on GitHub today and later hosted through MkDocs, Docusaurus, VitePress, or another static documentation system with minimal migration.

## Documentation Layout

### Foundation

- [Foundation](foundation.md) explains why Holy Blocker exists, the Christian rationale behind its design decisions, and what "covenant with my eyes" requires technically.

### Design documents

- [Architecture](architecture.md) describes the product shape, local-first privacy model, and major runtime components including the proposed workspace layout.
- [Network Pipeline](network-pipeline.md) describes the five-phase network interception path: packet filtering, TLS MITM, text scanning, image sandboxing, and video stream watchdog.
- [Edge Daemons](edge-daemons.md) records the Windows and Android daemon strategy, including the Windows foreground scanning loop and event hooks.
- [Content Classification](content-classification.md) describes how image classification, OCR, and text policy decisions should work together.
- [Evaluation and CI](evaluation-and-ci.md) describes how to test the blocker reliably without putting sensitive corpora in the public repository.
- [Security Backlog](security-backlog.md) prioritizes the repository's secure SDLC, CI/CD, trust-boundary hardening, and release-integrity work.
- [../CHANGELOG.md](../CHANGELOG.md) records completed changes and infrastructure milestones.

### Implementation plans

Each plan lists the current state of the package, the next modules to add (with types and responsibilities), the implementation order, and explicit deferrals. Read the plan before starting work on a package.

- [text-policy/PLAN.md](text-policy/PLAN.md) — Rust text-policy engine (normalize + lexicon done; scorer/evaluator/policy next)
- [proxy/PLAN.md](proxy/PLAN.md) — Rust MITM proxy (HTTP forwarding done; TLS termination + phase routing next)
- [win-daemon/PLAN.md](win-daemon/PLAN.md) — C++ Windows daemon (event hooks done; capture/scanner/IPC next)
- [machine-learning/PLAN.md](machine-learning/PLAN.md) — Python ML pipeline (model + ONNX export skeleton; dataset/eval/TFLite next)
- [desktop/PLAN.md](desktop/PLAN.md) — Electron control panel (status UI done; daemon IPC + policy settings next)
- [net-shield/PLAN.md](net-shield/PLAN.md) — Rust TUN packet filter (not yet started)
- [image-sandbox/PLAN.md](image-sandbox/PLAN.md) — Rust perceptual hash + ONNX image classifier (not yet started)
- [video-watchdog/PLAN.md](video-watchdog/PLAN.md) — Rust async video segment sampler (not yet started)

## Format Decision

The documentation source of truth is Markdown under `docs/`. Keep docs generator-neutral unless a hosting tool is selected later. Prefer relative links between pages and avoid embedding sensitive blocklists, explicit eval cases, private datasets, or generated adult-content screenshots in this public documentation tree.

