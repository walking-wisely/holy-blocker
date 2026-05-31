# Holy Blocker Documentation

This directory contains the project documentation for Holy Blocker. The files are plain Markdown by design so they can be rendered directly on GitHub today and later hosted through MkDocs, Docusaurus, VitePress, or another static documentation system with minimal migration.

## Documentation Layout

- [Architecture](architecture.md) describes the product shape, local-first privacy model, and major runtime components.
- [Edge Daemons](edge-daemons.md) records the Windows and Android daemon strategy, including the Windows foreground scanning loop and event hooks.
- [Content Classification](content-classification.md) describes how image classification, OCR, and text policy decisions should work together.
- [Evaluation and CI](evaluation-and-ci.md) describes how to test the blocker reliably without putting sensitive corpora in the public repository.

## Format Decision

The documentation source of truth is Markdown under `docs/`. Keep docs generator-neutral unless a hosting tool is selected later. Prefer relative links between pages and avoid embedding sensitive blocklists, explicit eval cases, private datasets, or generated adult-content screenshots in this public documentation tree.

