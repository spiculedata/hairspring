import unittest

from hairspring import _catalog as catalog
from hairspring._catalog import Entry
from hairspring._kinds import Kind


def basic() -> list[Entry]:
    return [
        Entry("basic.always.ok", Kind.ALWAYS),
        Entry("basic.always.bad", Kind.ALWAYS),
        Entry("basic.sometimes", Kind.SOMETIMES),
        Entry("basic.reach.hit", Kind.REACHABLE),
        Entry("basic.reach.never", Kind.REACHABLE),
        Entry("basic.unreach.clean", Kind.UNREACHABLE),
        Entry("basic.aou.never", Kind.ALWAYS_OR_UNREACHABLE),
    ]


class KindTests(unittest.TestCase):
    def test_kind_numbers_are_the_c_sdks(self) -> None:
        self.assertEqual(
            [int(k) for k in Kind],
            [1, 2, 3, 4, 5],
        )

    def test_names_and_flags(self) -> None:
        self.assertEqual(Kind.ALWAYS_OR_UNREACHABLE.wire_name, "always_or_unreachable")
        self.assertEqual(
            [k.must_hit for k in Kind],
            [True, False, True, True, False],
        )
        self.assertEqual(
            [k.is_point for k in Kind],
            [False, False, False, True, True],
        )


class HashTests(unittest.TestCase):
    def test_the_hash_equals_the_c_sdks_for_the_same_ids_and_kinds(self) -> None:
        # Printed by the C SDK's basic scenario (sdks/c/tests/fixtures/hs_sample.c): "catalog_hash":"fb6cf21a75aad6ef".
        self.assertEqual(catalog.catalog_hash(basic()), 0xFB6CF21A75AAD6EF)

    def test_the_hash_does_not_depend_on_order(self) -> None:
        self.assertEqual(
            catalog.catalog_hash(list(reversed(basic()))), catalog.catalog_hash(basic())
        )

    def test_an_id_declared_with_two_kinds_joins_both_pairs_to_the_hash(self) -> None:
        one = [Entry("c", Kind.ALWAYS)]
        two = [Entry("c", Kind.ALWAYS), Entry("c", Kind.SOMETIMES)]
        self.assertNotEqual(catalog.catalog_hash(one), catalog.catalog_hash(two))

    def test_an_empty_catalog_hashes_to_zero(self) -> None:
        self.assertEqual(catalog.catalog_hash([]), 0)


class CountTests(unittest.TestCase):
    def test_distinct_ids_ignore_the_kind(self) -> None:
        entries = [Entry("c", Kind.ALWAYS), Entry("c", Kind.SOMETIMES), Entry("d", Kind.REACHABLE)]
        self.assertEqual(catalog.distinct_ids(entries), 2)


if __name__ == "__main__":
    unittest.main()
