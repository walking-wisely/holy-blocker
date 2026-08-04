# Review Triggers

The router. Match the changed paths against this table to decide **whether** the security aspect
applies and **which** boundary rules to load. Most changes match zero or one row.

A change matching no row has no security surface. Say so and stop.

| Boundary | Paths | Why |
|---|---|---|
| Untrusted input parsers | `packages/net-shield/src/{dns,udp,sni,tun,radix}.rs`, `packages/net-shield-ffi/**`, `packages/mitm-proxy/src/{connect,forward,tunnel,proxy}.rs` | Attacker-controlled bytes off a wire or TUN device |
| TLS interception and CA | `packages/mitm-proxy/src/tls.rs`, `native-modules/mac-daemon/Sources/MacDaemon/{CATrust,FirefoxTrust}.swift`, any Windows trust-store code | A root CA in the system trust store; the highest-value target in the repo |
| Privilege boundaries | `native-modules/mac-daemon/Sources/MacDaemon/{PrivilegedCommand,LaunchdJob,CodeSigning,ProxyConfiguration,NetworkServices}.swift`, `native-modules/win-daemon/**`, `native-modules/win-network/**`, anything invoking a shell or elevating | user → root crossings, and command construction |
| IPC | `apps/desktop/src/preload/**`, `apps/desktop/src/main/{daemon-ipc,ipc-handlers}.ts`, `native-modules/mac-daemon/Sources/MacDaemon/DaemonIPC.swift`, named-pipe code in `win-daemon`, exported Android components in `apps/mobile/app/src/main/AndroidManifest.xml` | Every message is untrusted input; local endpoints need ACLs |
| Tamper resistance and permissions | `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/policy/{ProtectionStore,ProtectionSchedule,TamperLog,GuardStatus,NetworkGuard}.kt`, `SettingsGuard*`, `*DeviceAdmin*`, `native-modules/mac-daemon/Sources/MacDaemon/PermissionGate.swift` | The guard is the product; a silent bypass is a finding |
| Content capture and sensitive data | `native-modules/mac-daemon/Sources/MacDaemon/{ScreenCapture,Scanner,ScanLoop,Overlay}.swift`, `apps/mobile/.../policy/{ScreenCapture,FrameGate}.kt`, `*FrameSink*`, `packages/image-sandbox/src/**`, any OCR or AX-text path | Frames, OCR text, URLs and titles must not reach disk, logs, or errors |
| Supply chain and model artifacts | `**/Cargo.toml`, `**/Cargo.lock`, `package.json`, `pnpm-lock.yaml`, `machine-learning/pyproject.toml`, `apps/mobile/**/build.gradle*`, `packages/image-sandbox/src/classifier.rs`, model-provisioning code | New runtime dependencies, and models loaded from writable locations |

## Also trigger on, regardless of path

- A new process, service, daemon, or thread that outlives a request.
- A new file written outside a temp directory, or any new persisted state.
- A new listening socket, port, or bind address.
- Anything that reads an environment variable to decide a security-relevant behaviour.
- A dependency added to a runtime (not dev) dependency list.
- Any change that makes a previously-failing path succeed — fail-open regressions are the quiet ones.

## Explicitly not a trigger

- `apps/desktop/src/renderer/**` styling, layout, and component structure.
- Documentation under `docs/`, except `docs/decisions/**` where a decision changes a boundary.
- Test files, unless they add a fixture containing real captured content.
- `machine-learning/**` training and evaluation code — it does not run on a user's machine. The
  *artifacts* it produces do, and those are covered by the supply-chain row.

## Cadence

Per PR, this router is the whole gate: match, apply the matched rules, done. The heavier passes
below are scheduled, not per-change.

| When | Pass |
|---|---|
| Every PR | Route and apply matched boundary rules |
| Per package, once | Audit mode — produce the baseline in `docs/engineering/security-backlog.md` |
| Monthly pre-launch | Re-read the boundary list for new flows the router does not yet cover |
| Every release | Backlog triage: nothing `[HIGH]` left `open` without an explicit accept |
| Quarterly post-launch | Dependency and platform-guidance refresh |
