# 4. `ProxyConfiguration` — pointing macOS at the proxy, and putting it back

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/ProxyConfiguration.swift
```

Responsibilities:

- For every enabled network service, set the web proxy and secure web proxy to the local
  `mitm-proxy` listener, and apply the bypass list.
- **Snapshot the prior state of every service before modifying it**, and restore that exact state on
  stop or uninstall. The daemon must leave the machine as it found it — including services that
  already had a proxy configured for unrelated reasons.
- Persist the snapshot to disk, because the daemon can be killed between configure and restore. A
  crash must not strand the user behind a dead proxy with no route back.

Key signatures:

```swift
struct ProxySnapshot: Codable, Equatable {
    let service: String
    let web: ProxySetting?
    let secureWeb: ProxySetting?
    let bypassDomains: [String]
}

struct ProxyConfiguration {
    init(runner: CommandRunner, snapshotPath: URL)

    func snapshot(services: [NetworkService]) throws -> [ProxySnapshot]
    func apply(host: String, port: Int, bypass: [String], to services: [NetworkService]) throws
    func restore() throws
}
```

Test first, with `FakeCommandRunner`:

- Argv construction for `-setwebproxy`, `-setsecurewebproxy`, `-setwebproxystate off`, and
  `-setproxybypassdomains`, including a service name with spaces passed as a single argument.
- Round-trip: `snapshot` → `apply` → `restore` issues commands that return each service to its
  captured state, with a service that had no prior proxy correctly ending up **off** rather than
  pointed at a stale server.
- Disabled services are skipped.
- `restore()` with a snapshot file left by a previous crashed run is honoured on next startup.

**`restore()` is not atomic per service.** Clearing a service takes two calls —
`-setwebproxy <svc> "" 0` to blank the host, then `-setwebproxystate <svc> off` — and the order
cannot be reversed, because blanking also switches the proxy *on*. A process death between the two
therefore leaves the service **enabled with an empty server**, which is worse than either endpoint
and can break browsing outright. Observed once during development, when the supervisor was killed
mid-restore.

The persisted snapshot is what makes this recoverable: it is not deleted until the restore
completes, so `proxy-restore` on the next start replays it and lands correctly. That is the crash
path working as designed, but a startup check that detects and repairs "enabled with an empty
server" would close the window properly, and is not yet written.

**Bypass list.** At minimum `localhost`, `127.0.0.1`, `*.local`, and the link-local range, so the
proxy never sits in front of loopback and Bonjour traffic. Captive-portal detection hosts belong
here too or the user cannot join a hotel network while protected.

## Reference documents

- [`networksetup(8)` man page](https://keith.github.io/xcode-man-pages/networksetup.8.html) — the
  authoritative reference for every subcommand used here. Note the argument order for
  `-setwebproxy <service> <domain> <port> <authenticated> <username> <password>`; the trailing
  authentication arguments are optional but positional.
- [Apple — `SystemConfiguration` framework](https://developer.apple.com/documentation/systemconfiguration)
  — the API `networksetup` is a front-end for. Relevant if shelling out proves too slow or fragile
  and the daemon moves to `SCPreferences` directly; also documents the
  `kSCPropNetProxies*` keys that appear in snapshots.
- [Apple — `CFNetwork` proxy support](https://developer.apple.com/documentation/cfnetwork/cfproxysupport)
  — defines *which* clients honour these system settings, which is the coverage question below.
