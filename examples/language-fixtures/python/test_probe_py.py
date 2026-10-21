"""Tests of the Python feasibility probe (`probe_py.py`): its command line and its one output line.

Run from the repository root:  python3 -m unittest discover -s examples/language-fixtures/python -p 'test_*.py'
"""

from __future__ import annotations

import ast
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

PROBE = Path(__file__).resolve().parent / "probe_py.py"
LINE = re.compile(
    r"^PROBE v1 lang=py lock_order=([0-9a-f]{16}) arrival=([0-9a-f]{16}) rand=([0-9a-f]{16}) "
    r"vtime_ns=(\d{20}) extra=([0-9a-f]{16})$"
)


def run(*args: str) -> subprocess.CompletedProcess[bytes]:
    env = {k: v for k, v in os.environ.items() if not k.startswith("HAIRSPRING_") and k != "PYTHONPATH"}
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    return subprocess.run(
        [sys.executable, "-I", "-B", str(PROBE), *args], capture_output=True, env=env, timeout=300, check=False
    )


class CommandLineTests(unittest.TestCase):
    def test_no_argument_and_the_switch_argument_run(self) -> None:
        for args in ((), ("--switch=1.0",)):
            done = run(*args)
            self.assertEqual(done.returncode, 0, (args, done.stderr))
            self.assertEqual(done.stderr, b"")

    def test_anything_else_is_a_usage_error(self) -> None:
        for args in (("buggy",), ("--switch=2.0",), ("--switch=1.0", "x"), ("",)):
            done = run(*args)
            self.assertEqual(done.returncode, 64, args)
            self.assertEqual(done.stdout, b"")
            self.assertIn(b"usage", done.stderr)


class OutputTests(unittest.TestCase):
    def test_exactly_one_stdout_line_in_the_contract_form(self) -> None:
        done = run()
        text = done.stdout.decode()
        self.assertEqual(text.count("\n"), 1, text)
        self.assertIsNotNone(LINE.match(text.rstrip("\n")), text)

    def test_the_random_value_differs_between_runs(self) -> None:
        a = LINE.match(run().stdout.decode().rstrip("\n"))
        b = LINE.match(run().stdout.decode().rstrip("\n"))
        self.assertIsNotNone(a)
        self.assertIsNotNone(b)
        self.assertNotEqual(a.group(3), b.group(3))  # type: ignore[union-attr]

    def test_the_virtual_time_is_the_elapsed_monotonic_time(self) -> None:
        done = run()
        match = LINE.match(done.stdout.decode().rstrip("\n"))
        elapsed = int(match.group(4))  # type: ignore[union-attr]
        self.assertGreater(elapsed, 0)
        self.assertLess(elapsed, 120 * 10**9)


class ContractTests(unittest.TestCase):
    source = PROBE.read_text()

    def test_it_has_no_sdk_and_reads_neither_the_seed_nor_proc_nor_the_cpu_count(self) -> None:
        tree = ast.parse(self.source)
        imported = {
            alias.name.split(".")[0] for node in ast.walk(tree) if isinstance(node, ast.Import) for alias in node.names
        } | {node.module.split(".")[0] for node in ast.walk(tree) if isinstance(node, ast.ImportFrom) and node.module}
        self.assertNotIn("hairspring", imported)
        for forbidden in ("HAIRSPRING_SEED", "/proc", "cpu_count", "sched_getaffinity"):
            self.assertNotIn(forbidden, self.source, forbidden)

    def test_it_uses_the_parts_the_probe_is_about(self) -> None:
        for needle in ("threading.Thread", "threading.Lock", "queue.Queue", "setswitchinterval", "monotonic_ns"):
            self.assertIn(needle, self.source, needle)


if __name__ == "__main__":
    unittest.main()
