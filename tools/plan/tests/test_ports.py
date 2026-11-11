import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from tools.plan import devrun, ports


def dead_pid():
    pid = 4_000_000
    while ports.pid_alive(pid):
        pid += 1
    return pid


class RegistryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.reg = ports.Registry(Path(self.tmp.name))

    def tearDown(self):
        self.tmp.cleanup()

    def test_two_reservations_get_distinct_ports(self):
        a = self.reg.reserve("wt-a", "desktop", os.getpid(), base=5200)
        b = self.reg.reserve("wt-b", "desktop", os.getpid(), base=5200)
        self.assertNotEqual(a, b)
        self.assertGreaterEqual(min(a, b), 5200)

    def test_same_owner_and_name_while_alive_is_refused(self):
        self.reg.reserve("wt-a", "desktop", os.getpid())
        with self.assertRaises(ports.PortError):
            self.reg.reserve("wt-a", "desktop", os.getpid())

    def test_dead_holder_is_pruned_and_port_reused(self):
        first = self.reg.reserve("wt-a", "desktop", dead_pid(), base=5200)
        second = self.reg.reserve("wt-b", "desktop", os.getpid(), base=5200)
        self.assertEqual(first, second)
        self.assertEqual(len(self.reg.live()), 1)

    def test_port_in_use_by_a_stranger_is_skipped(self):
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            s.listen()
            taken = s.getsockname()[1]
            got = self.reg.reserve("wt-a", "desktop", os.getpid(), base=taken)
        self.assertGreater(got, taken)

    def test_release_frees_the_port(self):
        port = self.reg.reserve("wt-a", "desktop", os.getpid())
        self.reg.release(port, os.getpid())
        self.assertEqual(self.reg.live(), {})

    def test_release_by_another_pid_is_ignored(self):
        port = self.reg.reserve("wt-a", "desktop", os.getpid())
        self.reg.release(port, os.getpid() + 1)
        self.assertIn(str(port), self.reg.live())

    def test_corrupt_registry_is_treated_as_empty(self):
        (Path(self.tmp.name) / "ports.json").write_text("{not json")
        self.assertIsInstance(self.reg.reserve("wt-a", "desktop", os.getpid()), int)


class DevrunTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["HB_STATE_DIR"] = self.tmp.name

    def tearDown(self):
        os.environ.pop("HB_STATE_DIR", None)
        self.tmp.cleanup()

    def test_child_sees_port_and_url_and_exit_code_is_passed_on(self):
        out = Path(self.tmp.name) / "out"
        code = devrun.main(
            ["web", "--owner", "wt-a", "--", "sh", "-c", f'echo "$PORT $DEV_URL" > {out}; exit 3']
        )
        port, url = out.read_text().split()
        self.assertEqual(code, 3)
        self.assertEqual(url, f"http://127.0.0.1:{port}")

    def test_port_is_released_after_the_child_exits(self):
        devrun.main(["web", "--owner", "wt-a", "--", "true"])
        self.assertEqual(ports.Registry(Path(self.tmp.name)).live(), {})

    def test_missing_command_is_a_usage_error(self):
        self.assertEqual(devrun.main(["web", "--owner", "wt-a"]), 2)

    def test_unknown_command_is_reported_not_raised(self):
        self.assertEqual(devrun.main(["web", "--owner", "wt-a", "--", "no-such-binary-xyz"]), 127)

    def test_terminating_devrun_takes_the_whole_process_tree_down(self):
        marker = Path(self.tmp.name) / "grandchild.pid"
        runner = subprocess.Popen(
            [sys.executable, "-m", "tools.plan.devrun", "web", "--owner", "wt-a", "--",
             "sh", "-c", f"sleep 60 & echo $! > {marker}; wait"],
            env={**os.environ, "HB_STATE_DIR": self.tmp.name},
        )
        for _ in range(100):
            if marker.exists() and marker.read_text().strip():
                break
            time.sleep(0.05)
        grandchild = int(marker.read_text())
        runner.terminate()
        runner.wait(timeout=10)
        for _ in range(100):
            if not ports.pid_alive(grandchild):
                break
            time.sleep(0.05)
        self.assertFalse(ports.pid_alive(grandchild))


if __name__ == "__main__":
    unittest.main()
