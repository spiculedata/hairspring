import unittest

from hairspring._kinds import Kind
from hairspring._sites import MAX_DISTINCT, SiteTable, check_id, check_msg, pack_vals


class CheckTests(unittest.TestCase):
    def test_an_id_is_one_to_128_bytes(self) -> None:
        check_id("a")
        check_id("x" * 128)
        for bad in ("", "x" * 129, "é" * 65):
            with self.assertRaises(ValueError, msg=repr(bad)):
                check_id(bad)

    def test_an_id_must_be_text(self) -> None:
        for bad in (None, 5, b"abc"):
            with self.assertRaises(TypeError, msg=repr(bad)):
                check_id(bad)  # type: ignore[arg-type]

    def test_a_message_is_at_most_512_bytes(self) -> None:
        check_msg("")
        check_msg("m" * 512)
        with self.assertRaises(ValueError):
            check_msg("m" * 513)
        with self.assertRaises(TypeError):
            check_msg(None)  # type: ignore[arg-type]


class PackTests(unittest.TestCase):
    def test_fewer_than_four_values_are_zero_filled(self) -> None:
        self.assertEqual(pack_vals(()), (0, 0, 0, 0))
        self.assertEqual(pack_vals((7, 8)), (7, 8, 0, 0))

    def test_a_fifth_value_is_an_error(self) -> None:
        with self.assertRaises(TypeError):
            pack_vals((1, 2, 3, 4, 5))

    def test_values_wrap_to_signed_64_bits_like_a_c_cast(self) -> None:
        self.assertEqual(pack_vals((2**63,))[0], -(2**63))
        self.assertEqual(pack_vals((2**64 + 5,))[0], 5)
        self.assertEqual(pack_vals((-1,))[0], -1)

    def test_booleans_and_index_types_count_as_integers(self) -> None:
        self.assertEqual(pack_vals((True, False)), (1, 0, 0, 0))

    def test_other_types_are_an_error(self) -> None:
        for bad in (1.5, "1", None):
            with self.assertRaises(TypeError, msg=repr(bad)):
                pack_vals((bad,))


class TableTests(unittest.TestCase):
    def test_the_same_id_and_kind_is_one_site(self) -> None:
        table = SiteTable()
        first = table.register(Kind.SOMETIMES, "m", "first", "a.py", 1)
        again = table.register(Kind.SOMETIMES, "m", "second", "b.py", 9)
        self.assertIs(first, again)
        self.assertEqual((again.msg, again.file, again.line), ("first", "a.py", 1))
        self.assertEqual(len(table.all()), 1)

    def test_lookup_finds_a_registered_site_without_registering(self) -> None:
        table = SiteTable()
        self.assertIsNone(table.lookup("m", Kind.SOMETIMES))
        site = table.register(Kind.SOMETIMES, "m", "", "a.py", 1)
        self.assertIs(table.lookup("m", Kind.SOMETIMES), site)
        self.assertIsNone(table.lookup("m", Kind.ALWAYS))

    def test_ids_that_differ_only_in_bytes_the_wire_replaces_merge(self) -> None:
        table = SiteTable()
        a = table.register(Kind.ALWAYS, "a b", "", "f.py", 1)
        b = table.register(Kind.ALWAYS, "a_b", "", "f.py", 2)
        self.assertIs(a, b)
        self.assertIs(table.lookup("a b", Kind.ALWAYS), a)
        self.assertEqual(a.id, "a_b")

    def test_one_id_with_two_kinds_is_two_sites_and_one_canonical(self) -> None:
        table = SiteTable()
        always = table.register(Kind.ALWAYS, "c", "", "f.py", 1)
        sometimes = table.register(Kind.SOMETIMES, "c", "", "f.py", 2)
        self.assertIsNot(always, sometimes)
        self.assertEqual(table.all(), (always, sometimes))
        self.assertEqual(table.canonical(), (always,))
        self.assertEqual(table.distinct_ids(), 1)

    def test_a_point_kind_knows_it_has_no_condition(self) -> None:
        table = SiteTable()
        self.assertTrue(table.register(Kind.REACHABLE, "r", "", "f.py", 1).point)
        self.assertFalse(table.register(Kind.ALWAYS, "a", "", "f.py", 1).point)

    def test_ids_beyond_the_index_are_refused_and_counted_by_the_caller(self) -> None:
        table = SiteTable()
        for i in range(MAX_DISTINCT):
            self.assertIsNotNone(table.register(Kind.REACHABLE, f"id.{i}", "", "f.py", 1))
        self.assertIsNone(table.register(Kind.REACHABLE, "one.more", "", "f.py", 1))
        # A second kind for a known id is still a conflict the engine must see, not an excess.
        self.assertIsNotNone(table.register(Kind.ALWAYS, "id.0", "", "f.py", 1))

    def test_a_new_site_starts_with_nothing_recorded(self) -> None:
        site = SiteTable().register(Kind.ALWAYS, "a", "", "f.py", 1)
        self.assertEqual((site.trues, site.falses), (0, 0))
        self.assertIsNone(site.first_true)
        self.assertIsNone(site.first_false)


if __name__ == "__main__":
    unittest.main()
