import sys
import unittest
from unittest import mock

import hairspring as hs
from hairspring._lines import EndReason

from .support import make_runtime


class PublicApiTests(unittest.TestCase):
    def setUp(self) -> None:
        self.rt, self.sink = make_runtime()
        patcher = mock.patch.object(hs, "_runtime", self.rt)
        patcher.start()
        self.addCleanup(patcher.stop)

    def report(self) -> None:
        self.rt.report(EndReason.EXIT, 0)

    def test_every_kind_of_assertion_is_available(self) -> None:
        hs.always(True, "a", "m")
        hs.always_or_unreachable(True, "b", "m")
        hs.sometimes(False, "c", "m")
        hs.reachable("d", "m")
        hs.unreachable("e", "m")
        self.report()
        kinds = {c["id"]: c["type"] for c in self.sink.of_kind("catalog")}
        self.assertEqual(
            kinds,
            {
                "a": "always",
                "b": "always_or_unreachable",
                "c": "sometimes",
                "d": "reachable",
                "e": "unreachable",
            },
        )

    def test_the_message_and_values_are_optional(self) -> None:
        hs.always(True, "a")
        hs.always(False, "b", "m", 1, 2, 3, 4)
        self.report()
        by_id = {a["id"]: a for a in self.sink.of_kind("assert")}
        self.assertEqual(by_id["b"]["first_false"]["vals"], [1, 2, 3, 4])

    def test_a_condition_is_judged_by_its_truth_value(self) -> None:
        hs.always([], "empty.list")
        hs.always("x", "text")
        hs.always(None, "none")
        self.report()
        by_id = {a["id"]: a for a in self.sink.of_kind("assert")}
        self.assertEqual((by_id["empty.list"]["falses"], by_id["text"]["trues"], by_id["none"]["falses"]), (1, 1, 1))

    def test_the_location_is_the_callers_whatever_the_entry_point(self) -> None:
        line = sys._getframe().f_lineno
        hs.always(True, "loc.always")
        hs.sometimes(True, "loc.sometimes")
        hs.reachable("loc.reachable")
        hs.unreachable("loc.unreachable")
        hs.always_or_unreachable(True, "loc.aou")
        hs.declare(hs.Kind.REACHABLE, "loc.declared")
        self.report()
        lines = {c["id"]: c["loc"]["line"] for c in self.sink.of_kind("catalog")}
        self.assertEqual(
            lines,
            {
                "loc.always": line + 1,
                "loc.sometimes": line + 2,
                "loc.reachable": line + 3,
                "loc.unreachable": line + 4,
                "loc.aou": line + 5,
                "loc.declared": line + 6,
            },
        )

    def test_work_thread_and_report_are_available(self) -> None:
        self.assertTrue(hs.thread("main", 0))
        hs.work(3)
        hs.report()
        self.assertEqual(self.sink.of_kind("end")[0]["reason"], "explicit")
        self.assertEqual(self.sink.of_kind("work")[0]["ops"], 3)
        self.assertEqual(self.sink.of_kind("thread")[0]["thread"], "main-0")

    def test_exit_records_the_status_and_raises_system_exit(self) -> None:
        with self.assertRaises(SystemExit) as caught:
            hs.exit(5)
        self.assertEqual(caught.exception.code, 5)
        self.rt.report(EndReason.EXIT, None)
        self.assertEqual(self.sink.of_kind("end")[0]["status"], 5)

    def test_bad_arguments_are_errors(self) -> None:
        with self.assertRaises(ValueError):
            hs.always(True, "")
        with self.assertRaises(TypeError):
            hs.always(True, 5)  # type: ignore[arg-type]
        with self.assertRaises(TypeError):
            hs.always(True, "a", "m", 1, 2, 3, 4, 5)
        with self.assertRaises(ValueError):
            hs.thread("", 0)

    def test_the_version_is_a_string_and_the_name_is_python(self) -> None:
        self.assertRegex(hs.__version__, r"^\d+\.\d+\.\d+")
        self.assertEqual(hs.SDK_NAME, "python")

    def test_the_module_exports_exactly_its_documented_names(self) -> None:
        self.assertEqual(
            sorted(hs.__all__),
            sorted(
                [
                    "Kind",
                    "SDK_NAME",
                    "__version__",
                    "always",
                    "always_or_unreachable",
                    "declare",
                    "exit",
                    "reachable",
                    "report",
                    "sometimes",
                    "thread",
                    "unreachable",
                    "work",
                ]
            ),
        )


if __name__ == "__main__":
    unittest.main()
