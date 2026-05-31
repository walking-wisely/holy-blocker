# Holy Blocker

Phase 1 scaffold for the Project Sanctuary roadmap:

- `apps/desktop`: Electron control panel for local logs and daemon status.
- `native-modules/win-daemon`: C++ Win32 event hook daemon skeleton.
- `machine-learning`: Python package for dataset preparation, training, and model export.

## Development

```powershell
pnpm install
pnpm dev:desktop
```

The native daemon and ML package are intentionally standalone so they can evolve without coupling desktop UI work to platform-level permissions or GPU/runtime setup.

## Documentation

Project documentation lives in [`docs/`](docs/README.md). The docs are plain Markdown so they can be hosted later with a static documentation generator without changing the source format.
