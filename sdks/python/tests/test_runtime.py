import os
import sys
import threading
import unittest
from typing import Any

from hairspring._kinds import Kind
from hairspring._lines import EndReason
from hairspring._runtime import Runtime

from .support import MemorySink, make_runtime, root_identity


def by_id(events: list[dict[str, Any]], kind: str) -> dict[str, dict[str, Any]]:
    return {e["id"]: e for e in events if e["kind"] == kind}


class ReportShapeTests(unittest.TestCase):
    def test_the_report_is_hello_catalog_asserts_work_threads_end_in_that_order(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.ALWAYS, True, "a", "m", (1, 2))
        rt.record(Kind.REACHABLE, True, "r", "", ())
        rt.work(5)
        self.assertTrue(rt.report(EndReason.EXIT, 0))
        kinds = [e["kind"] for e in sink.events()]
        self.assertEqual(kinds, ["hello", "catalog", "catalog", "assert", "assert", "work", "thread", "end"])

    def test_counts_and_witnesses(self) -> None:
        rt, sink = make_runtime()
        for i in range(5):
            rt.record(Kind.ALWAYS, True, "ok", "holds", (i, 2))
        rt.record(Kind.ALWAYS, False, "bad", "broken", (7, 8))
        for i in range(4):
            rt.record(Kind.SOMETIMES, i % 2 == 0, "some", "", (i,))
        rt.report(EndReason.EXIT, 0)
        asserts = by_id(sink.events(), "assert")
        self.assertEqual((asserts["ok"]["hits"], asserts["ok"]["trues"], asserts["ok"]["falses"]), (5, 5, 0))
        self.assertEqual(asserts["ok"]["first_true"], {"thread": "t#1", "ord": 1, "vals": [0, 2, 0, 0]})
        self.assertIsNone(asserts["ok"]["first_false"])
        self.assertEqual(asserts["bad"]["first_false"], {"thread": "t#1", "ord": 6, "vals": [7, 8, 0, 0]})
        self.assertIsNone(asserts["bad"]["first_true"])
        self.assertEqual(asserts["some"]["first_true"]["vals"], [0, 0, 0, 0])
        self.assertEqual(asserts["some"]["first_false"]["ord"], 8)
        self.assertEqual(asserts["some"]["first_false"]["vals"], [1, 0, 0, 0])

    def test_the_first_witness_is_kept_not_the_last(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.ALWAYS, False, "x", "", (1,))
        rt.record(Kind.ALWAYS, False, "x", "", (2,))
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("assert")[0]["first_false"]["vals"], [1, 0, 0, 0])

    def test_end_counts_what_the_threads_counted(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.ALWAYS, True, "a", "", ())
        rt.record(Kind.ALWAYS, True, "b", "", ())
        rt.report(EndReason.EXIT, 3)
        end = sink.of_kind("end")[0]
        self.assertEqual((end["sites"], end["evals"], end["threads"], end["dropped"]), (2, 2, 1, 0))
        self.assertEqual((end["reason"], end["status"]), ("exit", 3))
        self.assertEqual(sink.of_kind("work")[0]["ops"], 0)

    def test_an_empty_catalog_still_reports_hello_work_and_end(self) -> None:
        rt, sink = make_runtime()
        rt.report(EndReason.EXIT, 0)
        self.assertEqual([e["kind"] for e in sink.events()], ["hello", "work", "end"])
        self.assertEqual(sink.of_kind("end")[0]["sites"], 0)

    def test_the_report_is_written_once(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.REACHABLE, True, "r", "", ())
        self.assertTrue(rt.report(EndReason.EXPLICIT, 0))
        count = len(sink.lines)
        self.assertFalse(rt.report(EndReason.EXIT, 0))
        self.assertEqual(len(sink.lines), count)
        self.assertEqual(sink.of_kind("end")[0]["reason"], "explicit")

    def test_the_status_recorded_by_exit_is_reported(self) -> None:
        rt, sink = make_runtime()
        rt.set_status(7)
        rt.report(EndReason.EXIT, None)
        self.assertEqual(sink.of_kind("end")[0]["status"], 7)

    def test_hello_carries_the_tag_and_stability(self) -> None:
        sink = MemorySink()
        rt = Runtime(root_identity("p1.s42", stable=False), sink)
        rt.report(EndReason.EXIT, 0)
        hello = sink.of_kind("hello")[0]
        self.assertEqual((hello["proc"], hello["tag_stable"], hello["sdk"]), ("p1.s42", False, "python"))
        self.assertIs(hello["signal_handlers"], False)

    def test_losses_before_the_report_are_counted_in_dropped(self) -> None:
        rt, sink = make_runtime(dropped=2)
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("end")[0]["dropped"], 2)

    def test_lines_that_cannot_be_written_are_counted(self) -> None:
        rt, sink = make_runtime(MemorySink(fail=True))
        rt.record(Kind.REACHABLE, True, "r", "", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.lines, [])
        # hello, catalog, assert, work, thread and end: six lines, none written.
        self.assertEqual(rt.dropped, 6)

    def test_a_lost_descriptor_is_counted_once_and_nothing_is_written(self) -> None:
        rt, sink = make_runtime(MemorySink(lost=True))
        rt.report(EndReason.EXIT, 0)
        end = sink.of_kind("end")[0]
        self.assertEqual(end["dropped"], 1)

    def test_a_forked_child_of_the_starting_process_writes_nothing(self) -> None:
        rt, sink = make_runtime(owner_pid=os.getpid() + 1)
        rt.record(Kind.REACHABLE, True, "r", "", ())
        self.assertFalse(rt.report(EndReason.EXIT, 0))
        self.assertEqual(sink.lines, [])


class MergeAndKindTests(unittest.TestCase):
    def test_one_id_evaluated_from_two_places_is_one_property(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.SOMETIMES, True, "m", "one id", (1,))
        rt.record(Kind.SOMETIMES, False, "m", "one id", (1,))
        rt.record(Kind.SOMETIMES, True, "m", "one id", (2,))
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(len(sink.of_kind("assert")), 1)
        a = sink.of_kind("assert")[0]
        self.assertEqual((a["hits"], a["trues"], a["falses"]), (3, 2, 1))
        self.assertEqual(sink.of_kind("end")[0]["sites"], 1)

    def test_a_point_kind_ignores_the_condition(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.REACHABLE, False, "r", "", ())
        rt.record(Kind.UNREACHABLE, False, "u", "", ())
        rt.report(EndReason.EXIT, 0)
        asserts = by_id(sink.events(), "assert")
        for name in ("r", "u"):
            self.assertEqual((asserts[name]["trues"], asserts[name]["falses"]), (1, 0))
            self.assertNotIn("first_false", asserts[name])

    def test_one_id_declared_with_two_kinds_gets_a_catalog_line_each_and_one_assert(self) -> None:
        rt, sink = make_runtime()
        rt.record(Kind.ALWAYS, True, "c", "", ())
        rt.record(Kind.SOMETIMES, True, "c", "", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual([c["type"] for c in sink.of_kind("catalog")], ["always", "sometimes"])
        self.assertEqual(len(sink.of_kind("assert")), 1)
        self.assertEqual(sink.of_kind("assert")[0]["id"], "c")
        self.assertEqual(sink.of_kind("end")[0]["sites"], 1)

    def test_a_declared_site_that_never_runs_is_catalogued_with_no_hits(self) -> None:
        rt, sink = make_runtime()
        rt.declare(Kind.REACHABLE, "never", "not reached")
        rt.record(Kind.REACHABLE, True, "hit", "", ())
        rt.report(EndReason.EXIT, 0)
        asserts = by_id(sink.events(), "assert")
        self.assertEqual(asserts["never"]["hits"], 0)
        self.assertIsNone(asserts["never"]["first_true"])
        self.assertEqual(by_id(sink.events(), "catalog")["never"]["must_hit"], True)

    def test_declaring_and_then_evaluating_is_one_site(self) -> None:
        rt, sink = make_runtime()
        rt.declare(Kind.SOMETIMES, "s", "m")
        rt.record(Kind.SOMETIMES, True, "s", "m", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(len(sink.of_kind("catalog")), 1)
        self.assertEqual(sink.of_kind("assert")[0]["hits"], 1)

    def test_the_catalog_hash_is_the_c_sdks_for_the_same_ids(self) -> None:
        rt, sink = make_runtime()
        for kind, name in (
            (Kind.ALWAYS, "basic.always.ok"),
            (Kind.ALWAYS, "basic.always.bad"),
            (Kind.SOMETIMES, "basic.sometimes"),
            (Kind.REACHABLE, "basic.reach.hit"),
            (Kind.REACHABLE, "basic.reach.never"),
            (Kind.UNREACHABLE, "basic.unreach.clean"),
            (Kind.ALWAYS_OR_UNREACHABLE, "basic.aou.never"),
        ):
            rt.declare(kind, name, "")
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("hello")[0]["catalog_hash"], "fb6cf21a75aad6ef")


class LocationTests(unittest.TestCase):
    def test_a_site_remembers_where_it_was_first_evaluated(self) -> None:
        rt, sink = make_runtime()
        line = sys._getframe().f_lineno + 1
        rt.record(Kind.ALWAYS, True, "here", "", ())
        rt.report(EndReason.EXIT, 0)
        loc = sink.of_kind("catalog")[0]["loc"]
        self.assertEqual(loc["file"][-len("test_runtime.py") :], "test_runtime.py")
        self.assertEqual(loc["line"], line)


class ValidationTests(unittest.TestCase):
    def test_a_bad_id_is_an_error_and_records_nothing(self) -> None:
        rt, sink = make_runtime()
        for bad in ("", "x" * 129):
            with self.assertRaises(ValueError):
                rt.record(Kind.ALWAYS, True, bad, "", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("catalog"), [])
        self.assertEqual(sink.of_kind("end")[0]["evals"], 0)

    def test_a_bad_value_count_or_type_is_an_error_and_counts_nothing(self) -> None:
        rt, sink = make_runtime()
        with self.assertRaises(TypeError):
            rt.record(Kind.ALWAYS, True, "a", "", (1, 2, 3, 4, 5))
        with self.assertRaises(TypeError):
            rt.record(Kind.ALWAYS, True, "a", "", ("x",))
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("end")[0]["evals"], 0)

    def test_work_must_be_a_non_negative_integer(self) -> None:
        rt, _ = make_runtime()
        for bad in (-1, 1.5, "1"):
            with self.assertRaises((ValueError, TypeError), msg=repr(bad)):
                rt.work(bad)  # type: ignore[arg-type]

    def test_ids_beyond_the_index_are_counted_as_dropped_evaluations(self) -> None:
        rt, sink = make_runtime()
        from hairspring._sites import MAX_DISTINCT

        for i in range(MAX_DISTINCT):
            rt.declare(Kind.REACHABLE, f"id.{i}", "")
        rt.record(Kind.REACHABLE, True, "over", "", ())
        rt.record(Kind.REACHABLE, True, "over", "", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("end")[0]["dropped"], 2)
        self.assertEqual(sink.of_kind("end")[0]["sites"], 4096)


class ThreadTests(unittest.TestCase):
    def test_a_thread_can_name_itself_and_the_key_appears_in_witnesses_and_thread_lines(self) -> None:
        rt, sink = make_runtime()
        self.assertTrue(rt.thread("writer", 3))
        rt.record(Kind.ALWAYS, False, "x", "", ())
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("assert")[0]["first_false"]["thread"], "writer-3")
        t = sink.of_kind("thread")[0]
        self.assertEqual((t["thread"], t["key_stable"], t["evals"]), ("writer-3", True, 1))

    def test_each_thread_has_its_own_ordinals_and_slot(self) -> None:
        rt, sink = make_runtime()

        def worker(index: int) -> None:
            rt.thread("w", index)
            for _ in range(3):
                rt.record(Kind.SOMETIMES, True, "each", "", (index,))

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        rt.report(EndReason.EXIT, 0)
        lines = sink.of_kind("thread")
        self.assertEqual(sorted(t["thread"] for t in lines), ["w-0", "w-1", "w-2", "w-3"])
        self.assertTrue(all(t["evals"] == 3 for t in lines))
        self.assertEqual(sink.of_kind("end")[0]["threads"], 4)
        witness = sink.of_kind("assert")[0]["first_true"]
        self.assertEqual(witness["ord"], 1)

    def test_a_refused_key_returns_false_and_keeps_the_claim_order_key(self) -> None:
        rt, sink = make_runtime()
        self.assertTrue(rt.thread("w", 1))
        results = []
        t = threading.Thread(target=lambda: results.append(rt.thread("w", 1)))
        t.start()
        t.join()
        self.assertEqual(results, [False])

    def test_more_threads_than_slots_share_the_last_and_are_counted(self) -> None:
        rt, sink = make_runtime()

        def one() -> None:
            rt.record(Kind.REACHABLE, True, "r", "", ())

        for _ in range(300):
            t = threading.Thread(target=one)
            t.start()
            t.join()
        rt.report(EndReason.EXIT, 0)
        end = sink.of_kind("end")[0]
        self.assertEqual(end["threads"], 256)
        self.assertEqual(end["dropped"], 300 - 255)
        self.assertEqual(sink.of_kind("assert")[0]["hits"], 300)
        self.assertEqual(end["evals"], 300)

    def test_work_is_summed_over_threads(self) -> None:
        rt, sink = make_runtime()
        rt.work(2)
        t = threading.Thread(target=lambda: rt.work(40))
        t.start()
        t.join()
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("work")[0]["ops"], 42)


class ReentrancyTests(unittest.TestCase):
    def test_a_call_made_while_the_same_thread_holds_the_lock_does_not_deadlock(self) -> None:
        """A signal handler, or a finaliser run by the garbage collector, can call the SDK while the interrupted code
        is inside the SDK's own critical section. That must cost accuracy at worst, never a hang."""
        rt, sink = make_runtime()
        done = threading.Event()

        def interrupted_call() -> None:
            with rt._lock:  # the interrupted code is in the middle of a record
                rt.record(Kind.REACHABLE, True, "from.handler", "", ())
                rt.work(1)
                rt.thread("handler", 0)
            done.set()

        worker = threading.Thread(target=interrupted_call, daemon=True)
        worker.start()
        self.assertTrue(done.wait(10), "the SDK deadlocked on its own lock")
        worker.join()
        rt.report(EndReason.EXIT, 0)
        self.assertEqual(sink.of_kind("assert")[0]["hits"], 1)


class ConcurrencyTests(unittest.TestCase):
    """The GIL does not make `count += 1` atomic: these run with a very small switch interval to prove the locking."""

    def setUp(self) -> None:
        self.previous = sys.getswitchinterval()
        sys.setswitchinterval(1e-6)
        self.addCleanup(sys.setswitchinterval, self.previous)

    def test_counts_are_exact_under_contention(self) -> None:
        rt, sink = make_runtime()
        per_thread, workers = 2000, 8

        def hammer(index: int) -> None:
            rt.thread("h", index)
            for i in range(per_thread):
                rt.record(Kind.ALWAYS, i % 4 != 0, "contended", "", (i,))
                rt.record(Kind.REACHABLE, True, "point", "", ())
                rt.work(1)

        threads = [threading.Thread(target=hammer, args=(i,)) for i in range(workers)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        rt.report(EndReason.EXIT, 0)
        asserts = by_id(sink.events(), "assert")
        total = per_thread * workers
        self.assertEqual(asserts["contended"]["hits"], total)
        self.assertEqual(asserts["contended"]["falses"], total // 4)
        self.assertEqual(asserts["point"]["hits"], total)
        self.assertEqual(sink.of_kind("work")[0]["ops"], total)
        end = sink.of_kind("end")[0]
        self.assertEqual(end["evals"], 2 * total)
        self.assertEqual(sum(t["evals"] for t in sink.of_kind("thread")), 2 * total)

    def test_a_snapshot_taken_while_threads_run_is_consistent(self) -> None:
        rt, _ = make_runtime()
        stop = threading.Event()

        def hammer(index: int) -> None:
            rt.thread("s", index)
            while not stop.is_set():
                rt.record(Kind.ALWAYS, index % 2 == 0, "snap", "", (index,))

        threads = [threading.Thread(target=hammer, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        try:
            for _ in range(300):
                snap = rt.snapshot()
                for row in snap.asserts:
                    # The protocol rejects a witness behind a zero count, and requires hits == trues + falses.
                    if row.first_true is not None:
                        self.assertGreater(row.trues, 0)
                    if row.first_false is not None:
                        self.assertGreater(row.falses, 0)
                hits = sum(r.trues + r.falses for r in snap.asserts)
                self.assertEqual(hits, sum(t.evals for t in snap.threads))
        finally:
            stop.set()
            for t in threads:
                t.join()

    def test_simultaneous_first_claims_get_distinct_slots(self) -> None:
        rt, sink = make_runtime()
        barrier = threading.Barrier(32)

        def go() -> None:
            barrier.wait()
            rt.record(Kind.REACHABLE, True, "r", "", ())

        threads = [threading.Thread(target=go) for _ in range(32)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        rt.report(EndReason.EXIT, 0)
        keys = sorted(t["thread"] for t in sink.of_kind("thread"))
        self.assertEqual(len(set(keys)), 32)


if __name__ == "__main__":
    unittest.main()
