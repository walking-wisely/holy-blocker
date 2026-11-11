"""Local TCP port reservations shared by every worktree.

The registry lives in ~/.holy-blocker/ports.json under a file lock, outside any checkout, so two
worktrees starting a dev server at the same moment cannot pick the same port before either binds it.
An entry is live while the process that reserved it is alive.
"""
import errno
import fcntl
import json
import os
import socket
from contextlib import contextmanager
from pathlib import Path

DEFAULT_BASE = 5200
SCAN_LIMIT = 800


class PortError(Exception):
    pass


def state_dir():
    return Path(os.environ.get("HB_STATE_DIR") or Path.home() / ".holy-blocker")


def pid_alive(pid):
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def is_free(port):
    for family, host in ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET, "0.0.0.0"), (socket.AF_INET6, "::1")):
        try:
            with socket.socket(family) as s:
                s.bind((host, port))
        except OSError as e:
            if family == socket.AF_INET6 and e.errno in (errno.EAFNOSUPPORT, errno.EADDRNOTAVAIL):
                continue
            return False
    return True


class Registry:
    def __init__(self, root=None):
        self.root = Path(root) if root else state_dir()
        self.file = self.root / "ports.json"
        self.lock = self.root / "ports.lock"

    @contextmanager
    def _locked(self):
        self.root.mkdir(parents=True, exist_ok=True)
        with open(self.lock, "w") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            yield

    def _load(self):
        try:
            data = json.loads(self.file.read_text())
        except (OSError, ValueError):
            return {}
        if not isinstance(data, dict):
            return {}
        return {
            p: e
            for p, e in data.items()
            if isinstance(e, dict) and {"owner", "name", "pid"} <= e.keys() and pid_alive(e["pid"])
        }

    def _save(self, entries):
        self.file.write_text(json.dumps(entries, indent=1))

    def live(self):
        with self._locked():
            return self._load()

    def reserve(self, owner, name, pid, base=DEFAULT_BASE):
        with self._locked():
            entries = self._load()
            for e in entries.values():
                if e["owner"] == owner and e["name"] == name:
                    raise PortError(f"{name} is already running for {owner} (pid {e['pid']})")
            for port in range(base, base + SCAN_LIMIT):
                if str(port) not in entries and is_free(port):
                    entries[str(port)] = {"owner": owner, "name": name, "pid": pid}
                    self._save(entries)
                    return port
            raise PortError(f"no free port in {base}-{base + SCAN_LIMIT - 1}")

    def release(self, port, pid):
        with self._locked():
            entries = self._load()
            if entries.get(str(port), {}).get("pid") == pid:
                del entries[str(port)]
            self._save(entries)
