# Win-Network — Implementation Plan

This document is the build plan for `native-modules/win-network/`: the privileged Windows Service that installs and manages the Wintun virtual adapter used by `packages/net-shield/`.

For the filtering logic that *runs on top of* this adapter, see [net-shield/PLAN.md](../net-shield/PLAN.md).  
For the overall packet pipeline, see [network-pipeline.md](../network-pipeline.md).

## Responsibility boundary

| Responsibility | Owner |
|---|---|
| Domain trie, CIDR filter, SNI parser | `packages/net-shield/` (Rust) |
| Packet read loop, block/allow/proxy dispatch | `packages/net-shield/` (Rust) |
| Wintun driver install / uninstall | `native-modules/win-network/` (C++20) |
| Virtual adapter create / destroy | `native-modules/win-network/` (C++20) |
| Windows routing rules (`netsh` / IP Helper API) | `native-modules/win-network/` (C++20) |
| Windows Service host (SCM registration, lifecycle) | `native-modules/win-network/` (C++20) |
| IPC server (named pipe) for desktop app commands | `native-modules/win-network/` (C++20) |
| Adapter lifetime across reboots and sleep/wake | `native-modules/win-network/` (C++20) |

`net-shield` never calls Win32 routing APIs. `win-network` never reads packet content.

## Current state

`native-modules/win-network/` does not exist. Nothing has been scaffolded — no CMakeLists.txt, no source files, no tests. This plan describes building it from scratch.

## Architecture: Windows Service + named pipe IPC

The service runs as `LocalSystem` (required for driver installation and routing table changes). The desktop app communicates with it over a named pipe:

```
\\.\pipe\HolyBlockerNetSvc
```

The desktop app (Electron, normal user session) sends JSON commands over the pipe; the service acknowledges with a JSON response. The service enforces that clients have the same machine SID — no cross-machine connections.

```
desktop app (user)
    │  named pipe JSON
    ▼
win-network service (LocalSystem)
    │  Wintun API
    ▼
Wintun virtual adapter  ──► net-shield read loop (Rust, LocalSystem)
    │  IP Helper API / netsh
    ▼
Windows routing table
```

`net-shield` is loaded in-process by the service (as a Rust DLL / staticlib) or run as a child process — the exact linkage is decided in the `net-shield` plan. Either way, the adapter handle is owned by the service and passed to the filter loop.

## Modules to add

### 1. `wintun_adapter` — driver install and adapter lifecycle

```
src/wintun_adapter.cpp
src/wintun_adapter.h
```

Responsibilities:

- `WintunAdapter::Install()` — loads `wintun.dll` dynamically (Wintun must be shipped alongside the installer; it is not in the Windows driver store by default). Creates the adapter with `WintunCreateAdapter`. Stores the adapter GUID in the registry under `HKLM\SOFTWARE\HolyBlocker\NetSvc` so it can be reclaimed on restart.
- `WintunAdapter::Open()` — called on service start if the adapter GUID already exists in the registry. Calls `WintunOpenAdapter`. Falls back to `Install()` if the GUID is stale.
- `WintunAdapter::StartSession()` — calls `WintunStartSession` with a 4 MB ring buffer and returns the session handle.
- `WintunAdapter::Close()` — calls `WintunEndSession` then `WintunCloseAdapter`. Called on service stop.
- `WintunAdapter::Uninstall()` — calls `WintunDeleteAdapter` and removes the registry key. Called by the uninstaller, not on every stop.
- No packet I/O happens here. The session handle is handed off to `net-shield`.

Key types:

```cpp
class WintunAdapter {
public:
    static std::expected<WintunAdapter, std::error_code> Install(std::wstring_view name);
    static std::expected<WintunAdapter, std::error_code> Open();

    WINTUN_SESSION_HANDLE StartSession();
    void Close();
    static void Uninstall();

private:
    WINTUN_ADAPTER_HANDLE handle_;
    GUID guid_;
};
```

### 2. `routing` — Windows routing table management

```
src/routing.cpp
src/routing.h
```

Responsibilities:

- `RoutingManager::AddDefaultRoute(adapter_luid)` — uses the IP Helper API (`CreateIpForwardEntry2`) to add a default route (0.0.0.0/0) through the Wintun adapter at a higher metric than the current default gateway. This makes all traffic flow through the adapter without removing internet access.
- `RoutingManager::RemoveDefaultRoute(adapter_luid)` — removes the route added above. Called on service stop and on uninstall.
- `RoutingManager::SetDnsServers(adapter_luid, servers)` — sets the adapter's DNS servers via `SetInterfaceDnsSettings` (Windows 10 20H1+). Used to prevent DNS leaks while the filter is active.
- `RoutingManager::ClearDnsServers(adapter_luid)` — restores automatic DNS on the adapter.
- All operations are idempotent: adding a route that already exists is a no-op; removing a route that is absent is a no-op.

### 3. `ipc_server` — named pipe command server

```
src/ipc_server.cpp
src/ipc_server.h
```

Responsibilities:

- Listens on `\\.\pipe\HolyBlockerNetSvc` with a DACL that allows access only to the `BUILTIN\Administrators` group and `NT AUTHORITY\SYSTEM`. Normal users cannot connect.
- Accepts one connection at a time (overlapped I/O). Each connection is a request/response exchange using newline-delimited JSON.
- Dispatches commands to a `CommandHandler` interface so the pipe layer can be tested independently.

Supported commands (first iteration):

| Command | Effect |
|---|---|
| `{"cmd":"start"}` | Install adapter if absent, add routing rules, start net-shield loop |
| `{"cmd":"stop"}` | Stop net-shield loop, remove routing rules, close adapter |
| `{"cmd":"status"}` | Returns `{"state":"running"|"stopped"|"error","error":"..."}` |
| `{"cmd":"reload_rules","path":"..."}` | Hot-reload filter rules from the given file path |

Responses always include `{"ok":true}` or `{"ok":false,"error":"..."}`.

### 4. `service_host` — Windows Service Control Manager integration

```
src/service_host.cpp
src/service_host.h
src/main.cpp
```

Responsibilities:

- `SERVICE_MAIN` entry point registered with the SCM. Handles `SERVICE_CONTROL_STOP`, `SERVICE_CONTROL_SHUTDOWN`, `SERVICE_CONTROL_POWEREVENT` (resume from sleep → re-check adapter state).
- Starts `IpcServer` on a background thread.
- On `SERVICE_CONTROL_STOP`: signals `IpcServer` to stop accepting connections, calls `RoutingManager::RemoveDefaultRoute`, calls `WintunAdapter::Close`.
- On `SERVICE_CONTROL_POWEREVENT` / `PBT_APMRESUMEAUTOMATIC`: calls `WintunAdapter::Open()` to reclaim the adapter (Wintun adapters survive sleep but the session handle must be refreshed).
- `main.cpp` handles two cases:
  - No arguments: calls `StartServiceCtrlDispatcher` (normal SCM launch).
  - `--install` / `--uninstall`: installs or removes the service via `CreateService` / `DeleteService` (called by the desktop installer with elevation).

### 5. `installer_actions` — install/uninstall helpers

```
src/installer_actions.cpp
src/installer_actions.h
```

Responsibilities:

- `InstallService(binary_path)` — calls `OpenSCManager` + `CreateService` with `SERVICE_AUTO_START`, `SERVICE_ERROR_NORMAL`, runs as `LocalSystem`. Sets the service description and a failure action (restart after 5 s for the first two failures, then no restart).
- `UninstallService()` — stops the service if running, then calls `DeleteService`.
- `CopyWintunDll(source_dir, system32)` — copies `wintun.dll` to `%SystemRoot%\System32` (required for `WintunCreateAdapter` to find the driver). Called during install.
- `RemoveWintunDll()` — removes the copy from System32. Called during uninstall if no other Wintun users are present.

These are called by the desktop app's installer (elevated NSIS / WiX script), not by the service itself at runtime.

## Testing strategy

Most bugs live in logic, not in whether the Wintun driver actually loaded. The test layers are designed so the common case — a normal dev machine or CI runner — covers the vast majority of behavior without elevation or hardware.

### Layer 0 — platform-independent logic (no Win32)

Pure C++ with no Win32 headers at all. Compiles and runs on Linux or macOS if needed, but more importantly runs on any Windows box in a normal user session.

Covers:
- JSON command parsing and response serialization (`ipc_server` command dispatch)
- DACL descriptor string construction (verify the SDDL string is correct before handing it to `ConvertStringSecurityDescriptorToSecurityDescriptor`)
- Registry key path formatting and adapter GUID round-tripping
- Service config struct construction (verify `CreateService` argument values before they reach Win32)
- `FilterAction` mapping and routing decision tables

These tests have zero Win32 imports. They live in `tests/test_logic.cpp` and link against nothing but the relevant source files and GoogleTest.

### Layer 1 — Win32-shimmed (no admin, no hardware)

The `fake_win32` static lib replaces real Win32 symbols at link time. Your production code (`wintun_adapter.cpp`, `routing.cpp`, etc.) calls the same function names it always would — but the implementations are fakes that record arguments and return configurable results.

CMake flag: `-DHOLY_BLOCKER_FAKE_WIN32=ON`

When this flag is set:
- `src/` files include `fake_win32/win32_api.h` instead of `<windows.h>` and the Wintun headers. The fake header declares identical signatures.
- The `net_svc_tests` target links `fake_win32` instead of the real Windows SDK import libs.
- `fake_win32/recorder.h` exposes a `FakeWin32::CallLog` singleton that tests can inspect and reset between cases.

Example test:

```cpp
TEST(RoutingManager, AddDefaultRouteCallsCreateIpForwardEntry2) {
    FakeWin32::CallLog::Reset();
    RoutingManager mgr;
    NET_LUID luid = {.Value = 0xABCD};

    auto result = mgr.AddDefaultRoute(luid);

    ASSERT_TRUE(result.has_value());
    auto& calls = FakeWin32::CallLog::Get();
    ASSERT_EQ(calls.CreateIpForwardEntry2.size(), 1u);
    EXPECT_EQ(calls.CreateIpForwardEntry2[0].row.InterfaceLuid.Value, 0xABCDu);
    EXPECT_EQ(calls.CreateIpForwardEntry2[0].row.DestinationPrefix.PrefixLength, 0u); // default route
}
```

What the shim covers:

| Real API | Fake behaviour |
|---|---|
| `WintunCreateAdapter` | Records name/GUID args; returns a fake `WINTUN_ADAPTER_HANDLE` |
| `WintunOpenAdapter` | Looks up GUID in a fake registry map; returns handle or `NULL` |
| `WintunStartSession` | Returns a fake `WINTUN_SESSION_HANDLE` |
| `WintunDeleteAdapter` | Records call; removes from fake registry map |
| `CreateIpForwardEntry2` | Records `MIB_IPFORWARD_ROW2`; returns configurable `NO_ERROR` or error code |
| `DeleteIpForwardEntry2` | Records args |
| `SetInterfaceDnsSettings` | Records args |
| `RegSetValueExW` / `RegGetValueW` | In-memory map; no actual registry touched |
| `OpenSCManager` / `CreateService` / `DeleteService` | Records args; returns fake handles |

This layer runs on any Windows machine in a standard user session — no UAC prompt, no Wintun driver, no routing table changes. GitHub Actions `windows-latest` runners qualify. All layer-0 and layer-1 tests are the `net_svc_tests` target and run in normal CI.

### Layer 2 — admin-required integration tests

A separate binary `net_svc_integration_tests.exe` built only when `-DHOLY_BLOCKER_INTEGRATION_TESTS=ON`. These tests genuinely touch the OS: they install the Wintun driver, add routing entries, register the service with the SCM.

Each test guards itself at the top:

```cpp
TEST(WintunAdapter, InstallAndOpen) {
    if (!IsUserAnAdministrator()) {
        GTEST_SKIP() << "requires elevation";
    }
    // ... real WintunCreateAdapter call ...
}
```

This means accidentally running the binary as a normal user produces `[ SKIPPED ]` output rather than failures. Running elevated on a dev machine or a self-hosted CI runner with admin access produces real results.

Integration tests are **not** run in standard CI. They are run:
- Manually before a significant merge that changes `wintun_adapter` or `routing`.
- On a self-hosted Windows runner (can be a local VM) when the adapter lifecycle code changes.

A self-hosted runner VM is the recommended setup: snapshot before the test run, restore after, so driver state never accumulates across runs.

### Layer 3 — manual smoke checklist

For behavior that can't be automated at any cost (does traffic actually flow? does the block survive sleep/wake?), a short human-run checklist before each milestone release:

- [ ] `holy_blocker_net_svc.exe --install` registers the service in SCM with `AUTO_START`.
- [ ] Service starts without error in Event Viewer.
- [ ] Sending `{"cmd":"start"}` over the pipe brings up the adapter (visible in `ipconfig /all`).
- [ ] A known-block domain is unreachable from the browser while the filter is running.
- [ ] Sleeping and resuming the machine leaves the adapter operational (check `ipconfig /all` after resume).
- [ ] Sending `{"cmd":"stop"}` removes the route and the adapter disappears from `ipconfig /all`.
- [ ] `holy_blocker_net_svc.exe --uninstall` removes the service from SCM and deletes `wintun.dll` from System32.

## Build system

```
native-modules/win-network/
  CMakeLists.txt
  src/
    main.cpp
    wintun_adapter.cpp / .h
    routing.cpp / .h
    ipc_server.cpp / .h
    service_host.cpp / .h
    installer_actions.cpp / .h
  tests/
    test_logic.cpp              ← Layer 0: no Win32
    test_routing.cpp            ← Layer 1: fake_win32 shim
    test_ipc_server.cpp         ← Layer 1: fake_win32 shim
    test_wintun_adapter.cpp     ← Layer 1: fake_win32 shim
    integration/
      test_wintun_install.cpp   ← Layer 2: admin-required, GTEST_SKIP guard
      test_routing_live.cpp     ← Layer 2: admin-required, GTEST_SKIP guard
  fake_win32/
    win32_api.h                 ← drop-in replacement for <windows.h> + wintun.h
    recorder.h / recorder.cpp   ← CallLog singleton
    fake_wintun.cpp
    fake_iphelper.cpp
    fake_registry.cpp
    fake_scm.cpp
  vendor/
    wintun/                     ← Wintun SDK headers (wintun.h) — DLL not checked in
```

CMake targets:

- `holy_blocker_net_svc` — the service executable.
- `net_svc_tests` — Layer 0 + Layer 1 tests. Built by default; links `fake_win32`. No elevation needed. Run in CI.
- `net_svc_integration_tests` — Layer 2 tests. Built only with `-DHOLY_BLOCKER_INTEGRATION_TESTS=ON`. Requires elevation at runtime.

Minimum Windows SDK: 10.0.19041.0 (required for `SetInterfaceDnsSettings`).

## Implementation order

1. ~~**Scaffold CMakeLists.txt** — targets, compiler flags (`/W4 /WX`, C++20), GoogleTest dependency via FetchContent, `fake_win32` shim lib.~~ **Done.**
2. **`wintun_adapter`** — implement registry persistence and `Open`/`Install`/`Close`. Unit-test with a stub `wintun.dll` loader that records calls.
3. **`routing`** — implement `AddDefaultRoute` / `RemoveDefaultRoute` / DNS setters against the IP Helper API. Unit-test with `fake_win32` shims that record `CreateIpForwardEntry2` calls.
4. **`ipc_server`** — implement the pipe listener and JSON dispatch against the `CommandHandler` interface. Unit-test command parsing and response serialization without opening a real pipe.
5. **`service_host`** — wire `WintunAdapter`, `RoutingManager`, `IpcServer`, and SCM callbacks together. Smoke-test by installing the service on a dev machine and issuing `{"cmd":"start"}` from a test client.
6. **`installer_actions`** — implement `InstallService` / `UninstallService` / DLL copy. Test by running `holy_blocker_net_svc.exe --install` in an elevated shell and confirming SCM registration.

## Microsoft documentation reference

Links are grouped by the module that uses them. Read these before implementing each module — the parameter gotchas and required access rights are non-obvious and not covered here.

### `wintun_adapter`

- [Wintun — Layer 3 TUN Driver for Windows](https://www.wintun.net/) — official site; API reference, integration guide, and pre-built binaries. The header (`wintun.h`) is the only file you need from here; the DLL is shipped with your installer, not checked in.

### `routing`

- [`CreateIpForwardEntry2` (netioapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-createipforwardentry2) — adds a route entry. Always call `InitializeIpForwardEntry` first to zero the struct before filling in only the fields you need.
- [`InitializeIpForwardEntry` (netioapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-initializeipforwardentry) — zeroes a `MIB_IPFORWARD_ROW2` to safe defaults. Skipping this is a common source of hard-to-reproduce routing bugs.
- [`MIB_IPFORWARD_ROW2` (netioapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/ns-netioapi-mib_ipforward_row2) — the route entry struct. Pay attention to `DestinationPrefix.PrefixLength` (0 = default route) and `Metric` (higher = lower priority; set above the existing default gateway metric so you don't break normal internet access).
- [`SetIpForwardEntry2` (netioapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-setipforwardentry2) — modifies an existing route in place; useful for metric adjustments without a delete/re-add cycle.
- [`SetInterfaceDnsSettings` (netioapi.h)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-setinterfacednssettings) — overrides DNS servers on a specific adapter. Requires Windows 10 20H1+ (build 19041). You must set the `Flags` field for every option you want applied and zero out the rest — partial structs produce silent no-ops.
- [`DNS_INTERFACE_SETTINGS` structure](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/ns-netioapi-dns_interface_settings3) — the settings struct passed to `SetInterfaceDnsSettings`; note the `Version` field must match the struct variant you use (`DNS_INTERFACE_SETTINGS_VERSION1` / `VERSION3`).

### `ipc_server`

- [Named Pipes overview](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes) — start here for the lifecycle: `CreateNamedPipe` → `ConnectNamedPipe` → read/write → `DisconnectNamedPipe` → loop or close.
- [`CreateNamedPipe` (winbase.h)](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea) — the pipe server creation function. The `lpSecurityAttributes` parameter is where you pass your DACL; `NULL` gives the default (Everyone can connect), which is wrong for a privileged service pipe.
- [Named Pipe Security and Access Rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights) — explains DACL interaction, the `FILE_CREATE_PIPE_INSTANCE` requirement for additional server instances, and why the default descriptor is too permissive for inter-process trust boundaries.

### `service_host`

- [Service Control Manager](https://learn.microsoft.com/en-us/windows/win32/services/service-control-manager) — overview of the SCM database, start types, and failure actions. Read before touching `CreateService` parameters.
- [`CreateService` (winsvc.h)](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-createservicea) — registers the service. Note `SERVICE_AUTO_START` vs `SERVICE_DEMAND_START`, the `lpServiceStartName` field (`NULL` = LocalSystem), and the failure action struct (`SERVICE_FAILURE_ACTIONS`) for restart-on-crash behaviour.
- [Service Control Programs](https://learn.microsoft.com/en-us/windows/win32/services/service-control-programs) — covers the `ServiceMain` entry point, `RegisterServiceCtrlHandlerEx`, `SetServiceStatus`, and the `SERVICE_CONTROL_POWEREVENT` handler used for sleep/wake recovery.

### Future: WFP callout driver (not Phase 1)

- [Windows Filtering Platform — start page](https://learn.microsoft.com/en-us/windows/win32/fwp/windows-filtering-platform-start-page)
- [WFP Architecture overview](https://learn.microsoft.com/en-us/windows/win32/fwp/windows-filtering-platform-architecture-overview) — understand Base Filtering Engine, shims, and callout drivers before writing any WFP code.
- [WFP API sets](https://learn.microsoft.com/en-us/windows/win32/fwp/api-sets) — index of all WFP functions grouped by area (filter management, session management, callout registration, etc.).

## What this does not cover

- **WFP callout driver** — kernel-mode packet filtering via Windows Filtering Platform is a future phase. The Wintun + userspace splice path in `net-shield` is sufficient for Phase 1.
- **QUIC / UDP blocking** — blocking QUIC traffic to force HTTP/2 fallback is a policy decision documented in [decisions/](../decisions/). Implementation is deferred.
- **macOS `NEPacketTunnelProvider`** — handled in `native-modules/mac-network/`. Same `PacketSink` trait from `net-shield` applies; the adapter layer is Swift.
- **Android VpnService** — handled in `native-modules/android-service/`. Out of scope here.
- **Rule file format and sync** — how filter rules are authored, packaged, and delivered to the service is part of the broader policy pipeline. The `reload_rules` IPC command accepts a file path; what writes that file is a separate concern.
