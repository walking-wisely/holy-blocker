"""Run a dev server on a port no other worktree holds.

  python3 -m tools.plan.devrun NAME [--base N] [--owner SLUG] -- COMMAND...
  python3 -m tools.plan.devrun list

The child gets PORT and DEV_URL (http://127.0.0.1:PORT). The port is released when it exits.
Expand $PORT and $DEV_URL inside a single-quoted `sh -c` string, so the child shell does it.
"""
import argparse
import os
import signal
import subprocess
import sys
from pathlib import Path

from tools.plan import ports


def owner_slug():
    out = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    return Path(out.stdout.strip()).name if out.returncode == 0 and out.stdout.strip() else Path.cwd().name


def list_ports():
    for port, e in sorted(ports.Registry().live().items(), key=lambda kv: int(kv[0])):
        print(f"{port}\t{e['owner']}\t{e['name']}\tpid {e['pid']}")
    return 0


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv == ["list"]:
        return list_ports()
    cmd = []
    if "--" in argv:
        split = argv.index("--")
        argv, cmd = argv[:split], argv[split + 1:]
    parser = argparse.ArgumentParser(prog="devrun")
    parser.add_argument("name")
    parser.add_argument("--base", type=int, default=ports.DEFAULT_BASE)
    parser.add_argument("--owner", default=None)
    args = parser.parse_args(argv)
    if not cmd:
        print("devrun: no command after --", file=sys.stderr)
        return 2

    registry = ports.Registry()
    try:
        port = registry.reserve(args.owner or owner_slug(), args.name, os.getpid(), base=args.base)
    except ports.PortError as e:
        print(f"devrun: {e}", file=sys.stderr)
        return 1
    env = {**os.environ, "PORT": str(port), "DEV_URL": f"http://127.0.0.1:{port}"}
    try:
        child = subprocess.Popen(cmd, env=env, start_new_session=True)
    except OSError as e:
        registry.release(port, os.getpid())
        print(f"devrun: {e}", file=sys.stderr)
        return 127
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda s, _f: os.killpg(child.pid, s))
    try:
        return child.wait()
    finally:
        registry.release(port, os.getpid())


if __name__ == "__main__":
    sys.exit(main())
