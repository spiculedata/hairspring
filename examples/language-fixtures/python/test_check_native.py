"""Tests of the native self-check's own logic, with made-up run results (no process is started here)."""

from __future__ import annotations

import json
import unittest

import check_native as check
from check_native import ContractError, RunResult

FIRED = 0
CLEAN = 1


def stdout_line(mode: str = "buggy", final: int = 4000) -> bytes:
    return f"LANGAT v1 lang=py mode={mode} final={final:020d} expect={4000:020d} order={0:016x} rand={1:016x}\n".encode()


def stderr_report(falses: int, ids: tuple[str, ...] = ("py.counter_exact", "py.all_started")) -> bytes:
    lines = []
    for id_ in ids:
        f = falses if id_ == "py.counter_exact" else 0
        event = {"v": 1, "kind": "assert", "proc": "p1", "id": id_, "hits": 1, "trues": 1 - f, "falses": f}
        lines.append("HAIRSPRING_EVENT " + json.dumps(event))
    return ("\n".join(lines) + "\n").encode()


def result(mode: str, final: int, falses: int, code: int = 0) -> RunResult:
    return RunResult(code, stdout_line(mode, final), stderr_report(falses))


class ObserveTests(unittest.TestCase):
    def test_a_clean_run_is_observed(self) -> None:
        self.assertEqual(check.observe(result("fixed", 4000, 0), "fixed"), check.Observation(4000, False))

    def test_a_lost_update_is_observed_as_fired(self) -> None:
        self.assertEqual(check.observe(result("buggy", 3100, 1), "buggy"), check.Observation(3100, True))

    def test_a_non_zero_exit_is_a_contract_violation(self) -> None:
        with self.assertRaisesRegex(ContractError, "exit status"):
            check.observe(result("buggy", 4000, 0, code=3), "buggy")

    def test_stdout_must_be_exactly_one_line_in_the_contract_form(self) -> None:
        for bad in (b"", b"hello\n", stdout_line() + stdout_line(), stdout_line().rstrip(b"\n"), stdout_line().upper()):
            with self.assertRaisesRegex(ContractError, "stdout"):
                check.observe(RunResult(0, bad, stderr_report(0)), "buggy")

    def test_the_mode_in_the_line_must_be_the_one_that_ran(self) -> None:
        with self.assertRaisesRegex(ContractError, "mode"):
            check.observe(result("fixed", 4000, 0), "buggy")

    def test_the_report_must_have_both_assert_lines(self) -> None:
        with self.assertRaisesRegex(ContractError, "py.all_started"):
            check.observe(RunResult(0, stdout_line(), stderr_report(0, ("py.counter_exact",))), "buggy")

    def test_the_detector_and_the_program_must_agree(self) -> None:
        with self.assertRaisesRegex(ContractError, "property false but final"):
            check.observe(result("buggy", 4000, 1), "buggy")
        with self.assertRaisesRegex(ContractError, "did not fire"):
            check.observe(result("buggy", 3999, 0), "buggy")


class SummaryTests(unittest.TestCase):
    def test_counts_and_the_final_values(self) -> None:
        obs = [check.Observation(f, f != 4000) for f in (4000, 3000, 2000, 4000)]
        s = check.summarise("buggy", obs)
        self.assertEqual((s.runs, s.fired, s.exact), (4, 2, 2))
        self.assertEqual((s.min_final, s.max_final), (2000, 4000))
        self.assertEqual(s.median_final, 3500)


class VerdictTests(unittest.TestCase):
    def summaries(self, buggy_fired: int, fixed_fired: int, fixed_exact: int, runs: int = 50) -> dict[str, check.Summary]:
        return {
            "buggy": check.Summary("buggy", runs, buggy_fired, runs - buggy_fired, 1, 2, 4000),
            "fixed": check.Summary("fixed", runs, fixed_fired, fixed_exact, 4000, 4000, 4000),
        }

    def test_all_good(self) -> None:
        status, _ = check.verdict(self.summaries(50, 0, 50), cpus=4)
        self.assertEqual(status, check.OK)

    def test_a_locked_twin_that_loses_an_update_fails(self) -> None:
        status, messages = check.verdict(self.summaries(50, 1, 49), cpus=4)
        self.assertEqual(status, check.FAILED)
        self.assertTrue(any("locked twin" in m for m in messages))

    def test_a_racy_program_that_never_fires_fails_on_several_cpus(self) -> None:
        status, _ = check.verdict(self.summaries(0, 0, 50), cpus=4)
        self.assertEqual(status, check.FAILED)

    def test_a_racy_program_that_never_fires_on_one_cpu_is_skipped(self) -> None:
        status, _ = check.verdict(self.summaries(0, 0, 50), cpus=1)
        self.assertEqual(status, check.SKIPPED)

    def test_a_skip_never_hides_a_failure_of_the_twin(self) -> None:
        status, _ = check.verdict(self.summaries(0, 1, 49), cpus=1)
        self.assertEqual(status, check.FAILED)


if __name__ == "__main__":
    unittest.main()
