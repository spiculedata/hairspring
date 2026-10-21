"""Tests of the Python lost-update fixture (`at_py.py`): its command line, its stdout line, its report, its shape.

Run from the repository root:  python3 -m unittest discover -s examples/language-fixtures/python -p 'test_*.py'
"""

from __future__ import annotations

import ast
import json
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "at_py.py"
SDK_SRC = HERE.parents[2] / "sdks" / "python" / "src"
LINE = re.compile(
    r"^LANGAT v1 lang=py mode=(buggy|fixed|wide) final=(\d{20}) expect=(\d{20}) order=([0-9a-f]{16}) rand=([0-9a-f]{16})$"
)
PREFIX = "HAIRSPRING_EVENT "
EXPECT = 4000
CLEARED = ("HAIRSPRING_PROC", "HAIRSPRING_PROC_PID", "HAIRSPRING_EVENTS", "HAIRSPRING_EVENTS_FILE")


def run(*args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    base = {k: v for k, v in os.environ.items() if k not in CLEARED}
    base["PYTHONDONTWRITEBYTECODE"] = "1"
    base.update(env or {})
    return subprocess.run(
        [sys.executable, "-I", "-B", str(FIXTURE), *args],
        capture_output=True,
        env=base,
        timeout=300,
        check=False,
    )


def events(stderr: bytes) -> list[dict[str, Any]]:
    return [
        json.loads(line[len(PREFIX) :])
        for line in stderr.decode().splitlines()
        if line.startswith(PREFIX)
    ]


def assert_line(stderr: bytes, id_: str) -> dict[str, Any]:
    return next(e for e in events(stderr) if e["kind"] == "assert" and e["id"] == id_)


class CommandLineTests(unittest.TestCase):
    def test_no_mode_is_a_usage_error(self) -> None:
        done = run()
        self.assertEqual(done.returncode, 64)
        self.assertEqual(done.stdout, b"")
        self.assertIn(b"usage", done.stderr)

    def test_the_report_of_a_usage_error_carries_the_status(self) -> None:
        done = run("racy")
        end = [e for e in events(done.stderr) if e["kind"] == "end"][0]
        self.assertEqual(end["status"], 64)

    def test_an_unknown_mode_or_extra_argument_is_a_usage_error(self) -> None:
        for args in (("racy",), ("buggy", "extra"), ("fixed", "--switch=2.0"), ("buggy", "--switch=1.0", "x"), ("",)):
            done = run(*args)
            self.assertEqual(done.returncode, 64, args)
            self.assertEqual(done.stdout, b"", args)

    def test_the_switch_argument_is_accepted(self) -> None:
        done = run("fixed", "--switch=1.0")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(LINE.match(done.stdout.decode().strip()))


class FixedModeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.done = run("fixed")

    def test_exits_zero_with_one_line_in_the_contract_form(self) -> None:
        self.assertEqual(self.done.returncode, 0, self.done.stderr)
        text = self.done.stdout.decode()
        self.assertEqual(text.count("\n"), 1)
        match = LINE.match(text.rstrip("\n"))
        self.assertIsNotNone(match, text)
        mode, final, expect, _, _ = match.groups()  # type: ignore[union-attr]
        self.assertEqual((mode, int(final), int(expect)), ("fixed", EXPECT, EXPECT))

    def test_the_property_holds_with_four_witness_values(self) -> None:
        a = assert_line(self.done.stderr, "py.counter_exact")
        self.assertEqual((a["hits"], a["falses"]), (1, 0))
        self.assertEqual(a["first_true"]["vals"], [EXPECT, EXPECT, 4, 1000])
        started = assert_line(self.done.stderr, "py.all_started")
        self.assertEqual((started["hits"], started["trues"]), (1, 1))

    def test_the_report_has_the_workers_and_the_work(self) -> None:
        ev = events(self.done.stderr)
        keys = sorted(e["thread"] for e in ev if e["kind"] == "thread")
        # The workers name themselves; the main thread, which evaluates the properties last, keeps its claim-order key.
        self.assertEqual(keys, ["t#5", "worker-0", "worker-1", "worker-2", "worker-3"])
        self.assertEqual([e for e in ev if e["kind"] == "work"][0]["ops"], EXPECT)
        end = [e for e in ev if e["kind"] == "end"][0]
        self.assertEqual((end["sites"], end["dropped"], end["reason"]), (2, 0, "exit"))

    def test_the_ids_are_exactly_the_specified_ones(self) -> None:
        ids = {e["id"] for e in events(self.done.stderr) if e["kind"] == "assert"}
        self.assertEqual(ids, {"py.counter_exact", "py.all_started"})


class BuggyModeTests(unittest.TestCase):
    def test_the_detector_and_the_line_agree(self) -> None:
        done = run("buggy")
        self.assertEqual(done.returncode, 0, "exit 0 whatever the property says")
        match = LINE.match(done.stdout.decode().rstrip("\n"))
        self.assertIsNotNone(match)
        final = int(match.group(2))  # type: ignore[union-attr]
        a = assert_line(done.stderr, "py.counter_exact")
        self.assertEqual(a["falses"] > 0, final != EXPECT)
        if final != EXPECT:
            self.assertEqual(a["first_false"]["vals"], [final, EXPECT, 4, 1000])

    def test_the_wide_arm_reports_its_mode(self) -> None:
        done = run("wide")
        self.assertEqual(done.returncode, 0, done.stderr)
        match = LINE.match(done.stdout.decode().rstrip("\n"))
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "wide")  # type: ignore[union-attr]


class ContractTests(unittest.TestCase):
    """Properties of the source that the specification pins down."""

    source = FIXTURE.read_text()
    tree = ast.parse(source)

    def test_it_reads_neither_the_seed_nor_proc_nor_the_cpu_count_nor_the_clock(self) -> None:
        for forbidden in ("HAIRSPRING_SEED", "/proc", "cpu_count", "sched_getaffinity", "time.time", "monotonic", "perf_counter"):
            self.assertNotIn(forbidden, self.source, forbidden)

    def test_the_parameters_are_the_specified_ones(self) -> None:
        constants = {
            node.targets[0].id: ast.literal_eval(node.value)
            for node in self.tree.body
            if isinstance(node, ast.Assign)
            and isinstance(node.targets[0], ast.Name)
            and isinstance(node.value, ast.Constant)
        }
        self.assertEqual(
            (constants["THREADS"], constants["ITERATIONS"], constants["WINDOW"], constants["WINDOW_WIDE"]),
            (4, 1000, 1000, 10000),
        )

    def _function(self, name: str) -> ast.FunctionDef:
        return next(n for n in ast.walk(self.tree) if isinstance(n, ast.FunctionDef) and n.name == name)

    def test_the_window_is_a_load_then_the_spin_then_a_store_and_nothing_else(self) -> None:
        loop = next(n for n in ast.walk(self._function("racy_iterations")) if isinstance(n, ast.For))
        window = [ast.unparse(stmt) for stmt in loop.body[:3]]
        self.assertEqual(window[0], "v = state.counter")
        self.assertTrue(window[1].endswith("= spin(spin_rounds)"), window[1])
        self.assertEqual(window[2], "state.counter = v + 1")
        # Anything after the store is outside the window: the sink and the SDK's work counter.
        after = [ast.unparse(stmt) for stmt in loop.body[3:]]
        self.assertEqual(after, ["sink.value = acc", "hs.work(1)"])

    def test_the_locked_loop_holds_the_lock_across_the_same_three_steps(self) -> None:
        loop = next(n for n in ast.walk(self._function("locked_iterations")) if isinstance(n, ast.For))
        with_stmt = loop.body[0]
        self.assertIsInstance(with_stmt, ast.With)
        body = [ast.unparse(stmt) for stmt in with_stmt.body]  # type: ignore[attr-defined]
        self.assertEqual(body[0], "v = state.counter")
        self.assertTrue(body[1].endswith("= spin(spin_rounds)"))
        self.assertEqual(body[2], "state.counter = v + 1")
        self.assertEqual([ast.unparse(s) for s in loop.body[1:]], ["sink.value = acc", "hs.work(1)"])

    def test_the_sdk_is_not_called_inside_the_spin(self) -> None:
        spin = self._function("spin")
        names = {n.id for n in ast.walk(spin) if isinstance(n, ast.Name)}
        self.assertNotIn("hs", names)

    def test_the_sdk_found_is_the_one_in_the_repository(self) -> None:
        self.assertTrue((SDK_SRC / "hairspring" / "__init__.py").is_file())


if __name__ == "__main__":
    unittest.main()
