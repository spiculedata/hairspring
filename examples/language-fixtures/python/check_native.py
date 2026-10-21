#!/usr/bin/env python3
"""Native self-check of the Python lost-update fixture (no Hermit, no engine).

    check_native.py [--runs N] [--python PATH] [--timeout SECONDS] [--control]

Runs `at_py.py buggy` and `at_py.py fixed` (and `wide`, informationally) N times each (default 50) and checks, for
every run, the whole contract: exit status 0; exactly one stdout line of the `LANGAT v1` form for the mode that ran;
the report has the assert lines of `py.counter_exact` and `py.all_started`; and the detector and the program agree
(the property is false exactly when the final value is not 4,000). Across the runs: the locked twin never fires and
always ends at 4,000, and the racy program fires at least once. Prints one summary line per mode, with how often the
race fired and the spread of the final values.

Exit status: 0 when everything holds, 1 when not, 64 for a usage error. There is no "skipped" status (77) as in the
C, C++ and Go checks: CPython's interpreter lock switches threads on one CPU as well, so the race needs no second CPU
and a racy program that never fires is a failure wherever it runs.

`--control` feeds the racy program in as the "locked twin": the check must then FAIL, or it checks nothing.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

OK = 0
FAILED = 1
USAGE = 64
EXPECTED = 4000
DEFAULT_RUNS = 50
DEFAULT_TIMEOUT_SECONDS = 300
FIXTURE = Path(__file__).resolve().parent / "at_py.py"
PREFIX = "HAIRSPRING_EVENT "
COUNTER_ID = "py.counter_exact"
STARTED_ID = "py.all_started"
LINE = re.compile(
    r"^LANGAT v1 lang=py mode=(?P<mode>\w+) final=(?P<final>\d{20}) expect=(?P<expect>\d{20}) "
    r"order=[0-9a-f]{16} rand=[0-9a-f]{16}$"
)
CLEARED_ENV = ("HAIRSPRING_PROC", "HAIRSPRING_PROC_PID", "HAIRSPRING_EVENTS", "HAIRSPRING_EVENTS_FILE")


class ContractError(Exception):
    """A run broke the fixture's contract."""


@dataclass(frozen=True)
class RunResult:
    returncode: int
    stdout: bytes
    stderr: bytes


@dataclass(frozen=True)
class Observation:
    final: int
    fired: bool


@dataclass(frozen=True)
class Summary:
    mode: str
    runs: int
    fired: int
    exact: int
    min_final: int
    median_final: float
    max_final: int


def _assert_lines(stderr: bytes) -> dict[str, dict[str, object]]:
    found: dict[str, dict[str, object]] = {}
    for line in stderr.decode("utf-8", "replace").splitlines():
        if line.startswith(PREFIX):
            event = json.loads(line[len(PREFIX) :])
            if event.get("kind") == "assert":
                found[str(event["id"])] = event
    return found


def observe(result: RunResult, mode: str) -> Observation:
    """Check one run against the contract and return what it showed."""
    if result.returncode != 0:
        raise ContractError(f"exit status {result.returncode}, want 0")
    text = result.stdout.decode("utf-8", "replace")
    lines = text.split("\n")
    match = LINE.match(lines[0]) if len(lines) == 2 and lines[1] == "" else None
    if match is None:
        raise ContractError(f"stdout is not the one contract line: {text!r}")
    if match["mode"] != mode:
        raise ContractError(f"the line says mode={match['mode']}, but {mode} ran")
    final = int(match["final"])
    asserts = _assert_lines(result.stderr)
    for id_ in (COUNTER_ID, STARTED_ID):
        if id_ not in asserts:
            raise ContractError(f"the protocol report lacks the assert line for {id_}")
    fired = int(asserts[COUNTER_ID]["falses"]) > 0  # type: ignore[call-overload]
    if fired and final == EXPECTED:
        raise ContractError(f"property false but final={final}")
    if not fired and final != EXPECTED:
        raise ContractError(f"final={final} but the property did not fire")
    return Observation(final, fired)


def summarise(mode: str, observations: Sequence[Observation]) -> Summary:
    finals = [o.final for o in observations]
    return Summary(
        mode=mode,
        runs=len(observations),
        fired=sum(1 for o in observations if o.fired),
        exact=sum(1 for o in observations if o.final == EXPECTED),
        min_final=min(finals),
        median_final=statistics.median(finals),
        max_final=max(finals),
    )


def verdict(summaries: dict[str, Summary]) -> tuple[int, list[str]]:
    """The exit status and the reasons, from the buggy and fixed summaries."""
    messages: list[str] = []
    fixed, buggy = summaries["fixed"], summaries["buggy"]
    if fixed.fired != 0 or fixed.exact != fixed.runs:
        messages.append("FAIL the locked twin lost an update")
    if buggy.fired == 0:
        messages.append(f"FAIL the racy program never fired in {buggy.runs} runs: the planted race is not reachable")
    return (FAILED if messages else OK), messages


def run_once(python: str, args: Sequence[str], timeout: float) -> RunResult:
    env = {k: v for k, v in os.environ.items() if k not in CLEARED_ENV}
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    done = subprocess.run(
        [python, "-I", "-B", str(FIXTURE), *args], capture_output=True, env=env, timeout=timeout, check=False
    )
    return RunResult(done.returncode, done.stdout, done.stderr)


def collect(python: str, mode: str, args: Sequence[str], runs: int, timeout: float) -> Summary:
    observations = []
    for number in range(1, runs + 1):
        try:
            observations.append(observe(run_once(python, args, timeout), mode))
        except (ContractError, subprocess.TimeoutExpired) as error:
            raise ContractError(f"{mode} run {number}: {error}") from error
    return summarise(mode, observations)


def describe(s: Summary, label: str) -> str:
    return (
        f"py {label}: property false in {s.fired} of {s.runs} native runs, final exact in {s.exact}; "
        f"final min/median/max {s.min_final}/{s.median_final:g}/{s.max_final}"
    )


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--control", action="store_true", help="use the racy program as the locked twin; must FAIL")
    parser.add_argument(
        "--switch-arm",
        action="store_true",
        help="also run `buggy --switch=1.0` and report how often it fires (informational, never checked)",
    )
    try:
        options = parser.parse_args(argv[1:])
    except SystemExit:
        return USAGE
    if options.runs < 1:
        print("--runs must be at least 1", file=sys.stderr)
        return USAGE
    fixed_args = ["buggy"] if options.control else ["fixed"]
    try:
        buggy = collect(options.python, "buggy", ["buggy"], options.runs, options.timeout)
        fixed = collect(options.python, "buggy" if options.control else "fixed", fixed_args, options.runs, options.timeout)
        wide = collect(options.python, "wide", ["wide"], options.runs, options.timeout)
        switch = (
            collect(options.python, "buggy", ["buggy", "--switch=1.0"], options.runs, options.timeout)
            if options.switch_arm
            else None
        )
    except ContractError as error:
        print(f"FAIL {error}", file=sys.stderr)
        return FAILED
    print(describe(buggy, "buggy"))
    print(describe(fixed, "fixed" if not options.control else "fixed (control: the racy program)"))
    print(describe(wide, "wide"))
    if switch is not None:
        print(describe(switch, "buggy --switch=1.0 (informational)"))
    status, messages = verdict({"buggy": buggy, "fixed": fixed})
    for message in messages:
        print(message, file=sys.stderr if message.startswith("FAIL") else sys.stdout)
    if options.control:
        if status == FAILED:
            print("control ok: the check rejected the racy program standing in for the twin")
            return OK
        print("FAIL the control was not rejected: the check cannot fail", file=sys.stderr)
        return FAILED
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv))
