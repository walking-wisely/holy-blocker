# 5. `ProxySupervisor` — running and monitoring the Rust proxy

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/ProxySupervisor.swift
```

Responsibilities:

- Launch the `mitm-proxy` binary as a child process bound to `127.0.0.1` on a chosen port.
- Health-check the listener before configuring system proxy settings — configuring first and
  launching second would black-hole all traffic during the gap.
- Restart on unexpected exit with backoff; if the proxy cannot be kept alive, **restore proxy
  settings** rather than leaving the machine pointed at a dead listener. Fail open on the network
  path, never fail closed into a broken network.
- Ordering guarantee, which is the part worth testing: configure only after healthy, and restore
  before terminating.

Model the ordering as a pure state machine (`stopped → starting → healthy → configured →
restoring`) so the transitions are unit-testable without spawning anything.

**A third ordering rule emerged while building this, and it is the least obvious of the three:
system proxy settings must be applied exactly once per run.** `ProxyConfiguration.apply()`
snapshots current settings before writing its own, so re-applying after a restart would capture
*our* proxy as the machine's prior state — and `restore()` would then pin the user to a dead local
port permanently, with the real prior settings gone. The machine therefore tracks whether this run
already owns a snapshot, and a restart that comes back healthy transitions straight to `configured`
without re-applying.

**`mitm-proxy` currently takes no command-line arguments.** It hardcodes `127.0.0.1:8080` and
loads its CA from the relative path `data/ca`, so the child's working directory is load-bearing and
the port is not actually selectable yet. `MitmProxyProcess` accepts an `arguments` array so this
side needs no change once the Rust binary grows a CLI, but until it does, `HOLY_BLOCKER_PROXY_PORT`
only changes where the supervisor *probes*, not where the proxy *binds*. Giving `mitm-proxy` a
`--port` and `--ca-dir` is a prerequisite for running on any other port.
