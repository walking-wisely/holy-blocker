# 3. `NetworkServices` — enumerating and parsing network services

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/NetworkServices.swift
```

macOS proxy settings are **per network service** (Wi-Fi, Ethernet, Thunderbolt Bridge, each VPN),
not global. Configuring the proxy means iterating every service. This module is the pure parsing
half of that job.

Responsibilities:

- Parse the output of `networksetup -listallnetworkservices` into structured values.
- Handle the two parsing hazards explicitly: the output begins with a **header line** ("An asterisk
  (*) denotes that a network service is disabled."), and **disabled services are prefixed with
  `*`**. Passing a `*`-prefixed name back to `networksetup` fails, so the prefix must be stripped
  and the disabled state retained separately.
- Parse `networksetup -getwebproxy <service>` output into current proxy state, so prior settings can
  be restored later.

Key types:

```swift
struct NetworkService: Equatable {
    let name: String
    let isEnabled: Bool
}

struct ProxySetting: Equatable {
    let isEnabled: Bool
    let server: String
    let port: Int
}

enum NetworkServices {
    static func parseServiceList(_ output: String) -> [NetworkService]
    static func parseProxySetting(_ output: String) -> ProxySetting?
}
```

Both parsers are **pure string functions** with no `CommandRunner` dependency — the easiest and
highest-value tests in this layer. Test against captured real output including: the header line,
disabled `*`-prefixed entries, service names containing spaces, an empty list, and malformed input.
