import unittest

from hairspring import _fmt
from hairspring import _lines as lines
from hairspring._kinds import Kind
from hairspring._lines import (
    AssertRow,
    CatalogRow,
    EndReason,
    LineTooLong,
    ProcInfo,
    ThreadRow,
    WitnessOut,
)

VERSION = lines.SDK_VERSION


def proc(**changes: object) -> ProcInfo:
    base = dict(
        tag="p1",
        tag_stable=True,
        signal_handlers=False,
        catalog_hash=0xFB6CF21A75AAD6EF,
        n_ids=7,
        dropped=0,
    )
    base.update(changes)
    return ProcInfo(**base)  # type: ignore[arg-type]


def w(n: int) -> str:
    return f"{n:>20}"


def wit(text: str) -> str:
    return f"{text:<{_fmt.WITNESS_WIDTH}}"


def text(line: bytes) -> str:
    return line.decode("ascii")


class HelloTests(unittest.TestCase):
    def test_hello_names_the_sdk_and_the_catalog_hash(self) -> None:
        self.assertEqual(
            text(lines.hello(proc())),
            'HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"python","sdk_version":"'
            + VERSION
            + '","proc":"p1","catalog_hash":"fb6cf21a75aad6ef","signal_handlers":false}\n',
        )

    def test_hello_flags_an_unstable_tag(self) -> None:
        line = text(lines.hello(proc(tag_stable=False)))
        self.assertTrue(line.endswith('"signal_handlers":false,"tag_stable":false}\n'), line)


class CatalogTests(unittest.TestCase):
    def test_catalog_carries_text_and_location(self) -> None:
        row = CatalogRow("basic.always.ok", Kind.ALWAYS, "always holds", "tests/hs_sample.py", 31)
        self.assertEqual(
            text(lines.catalog(proc(), row)),
            'HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"basic.always.ok",'
            '"type":"always","must_hit":true,"msg":"always holds",'
            '"loc":{"file":"tests/hs_sample.py","line":31}}\n',
        )

    def test_must_hit_is_false_for_unreachable_and_always_or_unreachable(self) -> None:
        for kind in (Kind.UNREACHABLE, Kind.ALWAYS_OR_UNREACHABLE):
            line = text(lines.catalog(proc(), CatalogRow("x", kind, "", "f.py", 1)))
            self.assertIn(f'"type":"{kind.wire_name}","must_hit":false', line)

    def test_the_tail_of_a_long_path_is_kept(self) -> None:
        long_path = "d" * 400 + "/tail.py"
        line = text(lines.catalog(proc(), CatalogRow("x", Kind.ALWAYS, "", long_path, 1)))
        self.assertIn('/tail.py"', line)
        self.assertNotIn("d" * 300, line)

    def test_an_oversized_text_costs_the_text_not_the_line(self) -> None:
        row = CatalogRow("i" * 128, Kind.ALWAYS, '"' * 512, '"' * 256, 1)
        line = lines.catalog(proc(tag="t" * 64), row, max_len=400)
        self.assertLessEqual(len(line), 400)
        self.assertIn(b'"id":"' + b"i" * 128, line)
        self.assertNotIn(b'"msg"', line)
        with self.assertRaises(LineTooLong):
            lines.catalog(proc(tag="t" * 64), row, max_len=100)


class AssertTests(unittest.TestCase):
    def test_a_condition_pads_both_witnesses(self) -> None:
        row = AssertRow(
            "basic.sometimes",
            Kind.SOMETIMES,
            trues=2,
            falses=2,
            first_true=WitnessOut("t#1", 7, (0, 0, 0, 0)),
            first_false=WitnessOut("t#1", 8, (1, 0, 0, 0)),
        )
        expected = (
            'HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"basic.sometimes",'
            f'"hits":{w(4)},"trues":{w(2)},"falses":{w(2)},'
            f'"first_false":{wit(f"""{{"thread":"t#1","ord":{w(8)},"vals":[1,0,0,0]}}""")},'
            f'"first_true":{wit(f"""{{"thread":"t#1","ord":{w(7)},"vals":[0,0,0,0]}}""")}}}\n'
        )
        self.assertEqual(text(lines.assert_line(proc(), row)), expected)

    def test_a_missing_witness_is_null_padded_to_the_same_width(self) -> None:
        true_only = AssertRow(
            "a", Kind.ALWAYS, 5, 0, WitnessOut("t#1", 1, (0, 2, 0, 0)), None
        )
        both = AssertRow(
            "a",
            Kind.ALWAYS,
            5,
            1,
            WitnessOut("t#1", 1, (0, 2, 0, 0)),
            WitnessOut("t#1", 99, (-1, -2, -3, -4)),
        )
        a, b = lines.assert_line(proc(), true_only), lines.assert_line(proc(), both)
        self.assertIn(f'"first_false":{wit("null")},"first_true":', text(a))
        self.assertEqual(len(a), len(b), "the length does not depend on whether a failure happened")

    def test_a_point_kind_sends_only_the_true_side(self) -> None:
        row = AssertRow(
            "basic.reach.hit", Kind.REACHABLE, 1, 9, WitnessOut("t#1", 11, (0, 0, 0, 0)), None
        )
        line = text(lines.assert_line(proc(), row))
        self.assertNotIn("first_false", line)
        self.assertIn(f'"hits":{w(1)},"trues":{w(1)},"falses":{w(0)}', line)

    def test_a_witness_is_never_sent_behind_a_zero_count(self) -> None:
        row = AssertRow(
            "a", Kind.ALWAYS, 0, 0, WitnessOut("t#1", 1, (0, 0, 0, 0)), WitnessOut("t#1", 2, (0, 0, 0, 0))
        )
        line = text(lines.assert_line(proc(), row))
        self.assertIn(f'"first_false":{wit("null")},"first_true":{wit("null")}}}', line)


class WorkThreadEndTests(unittest.TestCase):
    def test_work_and_end_use_fixed_fields(self) -> None:
        self.assertEqual(
            text(lines.work(proc(), 300)),
            f'HAIRSPRING_EVENT {{"v":1,"kind":"work","proc":"p1","ops":{w(300)}}}\n',
        )
        self.assertEqual(
            text(lines.end(proc(), EndReason.EXIT, 0, 11, 1)),
            'HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit",'
            f'"status":{w(0)},"sites":7,"evals":{w(11)},"dropped":{w(0)},"threads":1}}\n',
        )

    def test_end_caps_sites_at_what_a_parser_accepts_and_names_the_reason(self) -> None:
        big = proc(n_ids=9000)
        line = text(lines.end(big, EndReason.SIGNAL, 11, 0, 0))
        self.assertIn('"reason":"signal"', line)
        self.assertIn('"sites":4096,', line)
        explicit = text(lines.end(big, EndReason.EXPLICIT, -3, 0, 0))
        self.assertIn(f'"reason":"explicit","status":{-3:>20}', explicit)

    def test_end_reports_the_drop_count_it_is_given(self) -> None:
        line = text(lines.end(proc(dropped=3), EndReason.EXIT, 0, 0, 0))
        self.assertIn(f'"dropped":{w(3)}', line)

    def test_thread_pads_the_key_and_the_stability_flag(self) -> None:
        unstable = text(lines.thread(proc(), ThreadRow("t#1", 11, 0, False)))
        self.assertEqual(
            unstable,
            'HAIRSPRING_EVENT {"v":1,"kind":"thread","proc":"p1","thread":"t#1"'
            + " " * (_fmt.THREAD_KEY_MAX - 3)
            + f',"evals":{w(11)},"tape_consumed":{w(0)},"key_stable":false}}\n',
        )
        stable = text(lines.thread(proc(), ThreadRow("w-1", 1, 0, True)))
        self.assertTrue(stable.endswith(',"key_stable":true }\n'))


class LimitTests(unittest.TestCase):
    def test_a_line_that_does_not_fit_is_refused_not_truncated(self) -> None:
        with self.assertRaises(LineTooLong):
            lines.hello(proc(tag="t" * 64), max_len=60)

    def test_every_line_stays_inside_the_cap_at_the_protocol_limits(self) -> None:
        row = AssertRow(
            "i" * 128,
            Kind.ALWAYS,
            2**64 - 1,
            2**64 - 1,
            WitnessOut("k" * 64, 2**64 - 1, (-(2**63),) * 4),
            WitnessOut("k" * 64, 2**64 - 1, (2**63 - 1,) * 4),
        )
        self.assertLessEqual(len(lines.assert_line(proc(tag="t" * 64), row)), 4096)


if __name__ == "__main__":
    unittest.main()
