"""End to end: run the sample programs as real processes and read what they write."""

import os
import subprocess
import sys
import tempfile
import unittest
from typing import Any

from .support import QUICKSTART, SAMPLE, clean_env, parse_events, run_sample, run_script

HASH_OF_BASIC = "fb6cf21a75aad6ef"


def of_kind(events: list[dict[str, Any]], kind: str) -> list[dict[str, Any]]:
    return [e for e in events if e["kind"] == kind]


def tags(events: list[dict[str, Any]]) -> list[str]:
    return sorted(e["proc"] for e in of_kind(events, "hello"))


class ReportTests(unittest.TestCase):
    def test_basic_report_is_complete(self) -> None:
        run = run_sample("basic")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(run.stdout, b"")
        events = run.events()
        self.assertEqual(events[0]["kind"], "hello")
        self.assertEqual(
            (events[0]["sdk"], events[0]["proc"], events[0]["catalog_hash"]),
            ("python", "p1", HASH_OF_BASIC),
        )
        end = of_kind(events, "end")[0]
        self.assertEqual((end["sites"], end["threads"], end["evals"], end["dropped"]), (7, 1, 11, 0))
        self.assertEqual(of_kind(events, "work")[0]["ops"], 300)
        self.assertEqual(events[-1]["kind"], "end")
        self.assertEqual(len(of_kind(events, "catalog")), 7)
        self.assertEqual(len(of_kind(events, "assert")), 7)

    def test_a_false_always_carries_its_witness(self) -> None:
        events = run_sample("basic").events()
        bad = next(e for e in of_kind(events, "assert") if e["id"] == "basic.always.bad")
        self.assertEqual(bad["first_false"], {"thread": "t#1", "ord": 6, "vals": [7, 8, 0, 0]})

    def test_the_location_is_the_sample_source(self) -> None:
        events = run_sample("basic").events()
        cat = next(e for e in of_kind(events, "catalog") if e["id"] == "basic.always.bad")
        self.assertTrue(cat["loc"]["file"].endswith("hs_sample.py"))
        self.assertGreater(cat["loc"]["line"], 0)

    def test_merged_ids_are_one_property(self) -> None:
        events = run_sample("merge").events()
        shared = next(e for e in of_kind(events, "assert") if e["id"] == "merge.shared")
        self.assertEqual((shared["hits"], shared["trues"], shared["falses"]), (4, 3, 1))
        self.assertEqual(of_kind(events, "end")[0]["sites"], 2)

    def test_an_id_with_two_kinds_is_reported_as_a_conflict_input(self) -> None:
        events = run_sample("conflict").events()
        self.assertEqual([c["type"] for c in of_kind(events, "catalog")], ["always", "sometimes"])
        self.assertEqual(len(of_kind(events, "assert")), 1)

    def test_a_program_with_no_assertions_still_reports_work(self) -> None:
        events = run_sample("empty").events()
        self.assertEqual([e["kind"] for e in events], ["hello", "work", "thread", "end"])
        self.assertEqual(events[1]["ops"], 1)

    def test_ids_and_messages_at_their_limits(self) -> None:
        events = run_sample("limits").events()
        ids = {c["id"]: c for c in of_kind(events, "catalog")}
        self.assertIn("x" * 128, ids)
        self.assertEqual(len(ids["x" * 128]["msg"]), 512)
        odd = ids["bad_id_with:odd/chars"]
        self.assertEqual(odd["msg"], 'quote " and backslash \\ in the message')


class ExitTests(unittest.TestCase):
    def test_exit_records_the_status_and_the_process_exits_with_it(self) -> None:
        run = run_sample("exit_status")
        self.assertEqual(run.returncode, 3)
        self.assertEqual(of_kind(run.events(), "end")[0]["status"], 3)

    def test_sys_exit_still_reports(self) -> None:
        run = run_sample("plain_exit")
        self.assertEqual(run.returncode, 7)
        end = of_kind(run.events(), "end")[0]
        self.assertEqual(end["reason"], "exit")
        self.assertEqual(end["sites"], 1)

    def test_an_uncaught_exception_still_reports(self) -> None:
        run = run_sample("raises")
        self.assertEqual(run.returncode, 1)
        self.assertEqual(of_kind(run.events(), "end")[0]["sites"], 1)

    def test_a_signal_handler_can_call_the_sdk(self) -> None:
        run = run_sample("signal_handler")
        self.assertEqual(run.returncode, 0, run.stderr)
        asserts = {a["id"]: a for a in of_kind(run.events(), "assert")}
        self.assertEqual(asserts["signal.handler"]["hits"], 4)
        self.assertEqual(asserts["signal.loop"]["hits"], 2000)

    def test_an_explicit_report_is_the_only_report(self) -> None:
        run = run_sample("explicit")
        events = run.events()
        self.assertEqual(len(of_kind(events, "hello")), 1)
        self.assertEqual(len(of_kind(events, "end")), 1)
        self.assertEqual(of_kind(events, "end")[0]["reason"], "explicit")
        self.assertEqual([a["id"] for a in of_kind(events, "assert")], ["explicit.before"])


class ThreadTests(unittest.TestCase):
    def test_a_hundred_threads_report_cleanly(self) -> None:
        events = run_sample("threads", "100").events()
        self.assertEqual(of_kind(events, "end")[0]["threads"], 101)
        w99 = [t for t in of_kind(events, "thread") if t["thread"] == "w-99"]
        self.assertEqual(len(w99), 1)
        self.assertTrue(w99[0]["key_stable"])

    def test_threads_beyond_the_table_are_capped_and_counted(self) -> None:
        events = run_sample("threads", "300").events()
        end = of_kind(events, "end")[0]
        self.assertEqual(end["threads"], 256)
        self.assertGreater(end["dropped"], 0)
        self.assertEqual(len(of_kind(events, "thread")), 256)


class ProcessTreeTests(unittest.TestCase):
    def test_a_forked_child_that_exits_leaves_one_clean_report(self) -> None:
        events = run_sample("fork").events()
        self.assertEqual(tags(events), ["p1"])
        self.assertEqual(len(of_kind(events, "end")), 1)
        parent = next(e for e in of_kind(events, "assert") if e["id"] == "fork.parent")
        self.assertEqual(parent["hits"], 2)

    def test_a_spawned_instrumented_child_takes_a_derived_unstable_tag(self) -> None:
        events = run_sample("spawn").events()
        found = tags(events)
        self.assertEqual(len(found), 2, found)
        self.assertEqual(found[0], "p1")
        self.assertTrue(found[1].startswith("p1.s"), found)
        for hello in of_kind(events, "hello"):
            spawned = hello["proc"].startswith("p1.s")
            self.assertEqual(hello.get("tag_stable") is False, spawned, hello)
        self.assertEqual(len(of_kind(events, "end")), 2)

    def test_a_multiprocessing_spawn_child_reports_under_its_own_tag(self) -> None:
        events = run_sample("mp_spawn").events()
        found = tags(events)
        self.assertEqual(len(found), 2, found)
        self.assertTrue(found[1].startswith("p1.s"), found)
        ids = {e["id"] for e in of_kind(events, "assert")}
        self.assertEqual(ids, {"mp.parent", "mp.child"})

    def test_a_multiprocessing_fork_child_leaves_the_parents_report_alone(self) -> None:
        events = run_sample("mp_fork").events()
        self.assertEqual(tags(events), ["p1"])
        ids = {e["id"] for e in of_kind(events, "assert")}
        self.assertEqual(ids, {"mp.parent"})

    def test_an_exec_before_the_report_leaves_one_report_under_the_original_tag(self) -> None:
        events = run_sample("execself").events()
        self.assertEqual(tags(events), ["p1"])
        self.assertEqual(of_kind(events, "end")[0]["sites"], 0)

    def test_an_exec_after_the_report_takes_an_x_tag(self) -> None:
        events = run_sample("exec_after_report").events()
        self.assertEqual(tags(events), ["p1", "p1.x1"])

    def test_two_processes_on_one_output_fold_cleanly(self) -> None:
        # macOS pipes are only atomic up to 512 bytes, so there the two processes share an O_APPEND events file.
        for _ in range(5):
            self.one_round_of_two_writers(use_file=not sys.platform.startswith("linux"))

    def one_round_of_two_writers(self, use_file: bool) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            events_path = os.path.join(tmp, "events")
            extra = {"HAIRSPRING_EVENTS_FILE": events_path} if use_file else {}
            read_end, write_end = os.pipe()
            procs = []
            for tag in ("pA", "pB"):
                env = clean_env({"HAIRSPRING_PROC": tag, **extra})
                procs.append(
                    subprocess.Popen(
                        [sys.executable, SAMPLE, "big"], env=env, stderr=write_end, stdout=subprocess.DEVNULL
                    )
                )
            os.close(write_end)
            if use_file:
                os.close(read_end)
            else:
                # Read to end of file before waiting: the report is larger than a pipe's buffer.
                with os.fdopen(read_end, "rb") as f:
                    data = f.read()
            for p in procs:
                self.assertEqual(p.wait(timeout=120), 0)
            if use_file:
                with open(events_path, "rb") as f:
                    data = f.read()
        events = parse_events(data)
        self.assertEqual(tags(events), ["pA", "pB"])
        asserts = of_kind(events, "assert")
        self.assertEqual(len(asserts), 128)
        self.assertTrue(all(a["hits"] == 3 for a in asserts))
        self.assertEqual(len(data.decode().splitlines()), len(events), "no line was split or glued")


class OutputTests(unittest.TestCase):
    def test_the_events_file_receives_the_report_instead_of_standard_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "events")
            run = run_sample("basic", env={"HAIRSPRING_EVENTS_FILE": path})
            self.assertEqual(run.stderr, b"")
            with open(path, "rb") as f:
                events = parse_events(f.read())
            self.assertEqual(of_kind(events, "end")[0]["dropped"], 0)
            self.assertEqual(len(of_kind(events, "assert")), 7)

    def test_an_events_file_that_cannot_be_opened_falls_back_to_standard_error_and_is_counted(self) -> None:
        run = run_sample("basic", env={"HAIRSPRING_EVENTS_FILE": "/nonexistent-dir/events"})
        self.assertEqual(of_kind(run.events(), "end")[0]["dropped"], 1)
        self.assertEqual(len(of_kind(run.events(), "assert")), 7)

    def test_events_off_writes_nothing(self) -> None:
        run = run_sample("basic", env={"HAIRSPRING_EVENTS": "off"})
        self.assertEqual((run.stderr, run.stdout, run.returncode), (b"", b"", 0))

    def test_the_process_tag_comes_from_the_environment(self) -> None:
        run = run_sample("exit_status", env={"HAIRSPRING_PROC": "p1.f3"})
        self.assertEqual(tags(run.events()), ["p1.f3"])

    def test_an_invalid_tag_is_refused_and_counted(self) -> None:
        run = run_sample("empty", env={"HAIRSPRING_PROC": "bad tag"})
        events = run.events()
        self.assertEqual(tags(events), ["p1"])
        self.assertEqual(of_kind(events, "end")[0]["dropped"], 1)

    def test_a_program_that_closes_stderr_and_reuses_it_does_not_receive_the_report(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "data")
            run = run_sample("closefd", data)
            self.assertEqual(run.returncode, 0, run.stderr)
            with open(data, "rb") as f:
                self.assertEqual(f.read(), b"application data\n")
        self.assertEqual(of_kind(run.events(), "end")[0]["dropped"], 0)

    def test_a_closed_stderr_pipe_does_not_kill_the_program_with_sigpipe(self) -> None:
        read_end, write_end = os.pipe()
        os.close(read_end)
        proc = subprocess.Popen(
            [sys.executable, SAMPLE, "sigpipe"], env=clean_env(), stderr=write_end, stdout=subprocess.DEVNULL
        )
        os.close(write_end)
        self.assertEqual(proc.wait(timeout=60), 0)

    def test_a_usage_error_exits_64(self) -> None:
        run = run_sample("no-such-scenario")
        self.assertEqual(run.returncode, 64)
        self.assertEqual(run.events()[0]["kind"], "hello")  # the SDK still reports an empty program


class QuickstartTests(unittest.TestCase):
    def test_the_quickstart_runs_and_reports(self) -> None:
        run = run_script(QUICKSTART)
        self.assertEqual(run.returncode, 0, run.stderr)
        events = run.events()
        ids = {a["id"]: a for a in of_kind(events, "assert")}
        self.assertEqual(
            set(ids), {"quickstart.recovery.ran", "quickstart.balance_non_negative", "quickstart.overdrawn"}
        )
        self.assertEqual(ids["quickstart.recovery.ran"]["hits"], 0)
        self.assertEqual(ids["quickstart.balance_non_negative"]["falses"], 1)
        self.assertEqual(of_kind(events, "end")[0]["threads"], 4)


class OverheadTests(unittest.TestCase):
    def test_importing_the_sdk_prints_nothing(self) -> None:
        done = subprocess.run(
            [sys.executable, "-c", "import hairspring"],
            env=clean_env(),
            capture_output=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(done.stdout, b"")
        self.assertEqual(done.returncode, 0)
        # Nothing was evaluated, but the report still says so once at exit.
        kinds = [e["kind"] for e in parse_events(done.stderr)]
        self.assertEqual(kinds, ["hello", "work", "end"])


if __name__ == "__main__":
    unittest.main()
