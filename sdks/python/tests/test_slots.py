import unittest

from hairspring._slots import MAX_THREADS, OVERFLOW_SLOT, SlotTable


class ClaimTests(unittest.TestCase):
    def test_threads_claim_slots_in_order_with_claim_order_keys(self) -> None:
        table = SlotTable()
        a, shared_a = table.claim()
        b, shared_b = table.claim()
        self.assertFalse(shared_a or shared_b)
        self.assertEqual((a.key, b.key), ("t#1", "t#2"))
        self.assertEqual((a.index, b.index), (0, 1))
        self.assertEqual(table.used(), 2)
        self.assertFalse(a.is_stable)

    def test_threads_past_the_table_share_the_last_slot_and_are_flagged(self) -> None:
        table = SlotTable()
        shared = sum(1 for _ in range(300) if table.claim()[1])
        self.assertEqual(table.used(), MAX_THREADS)
        self.assertEqual(shared, 300 - OVERFLOW_SLOT)
        self.assertEqual(table.key_at(OVERFLOW_SLOT), "t#overflow")
        self.assertEqual(table.key_at(254), "t#255")
        self.assertEqual(len(table.views()), MAX_THREADS)

    def test_the_slot_a_claim_returns_is_the_table_entry(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        slot.evals += 3
        self.assertEqual(table.views()[0].evals, 3)


class KeyTests(unittest.TestCase):
    def test_a_thread_can_name_itself(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        self.assertTrue(table.set_key(slot, "writer", 3))
        self.assertEqual(slot.key, "writer-3")
        self.assertTrue(slot.is_stable)

    def test_label_bytes_outside_the_alphabet_become_underscores(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        self.assertTrue(table.set_key(slot, "a b#c", 0))
        self.assertEqual(slot.key, "a_b_c-0")

    def test_a_key_cannot_be_taken_twice_but_can_be_retaken_by_its_owner(self) -> None:
        table = SlotTable()
        a, _ = table.claim()
        b, _ = table.claim()
        self.assertTrue(table.set_key(a, "w", 1))
        self.assertFalse(table.set_key(b, "w", 1))
        self.assertEqual(b.key, "t#2", "a refused key leaves the claim-order key")
        self.assertFalse(b.is_stable)
        self.assertTrue(table.set_key(a, "w", 1))

    def test_empty_and_overlong_labels_are_refused_with_an_error(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        with self.assertRaises(ValueError):
            table.set_key(slot, "", 0)
        with self.assertRaises(ValueError):
            table.set_key(slot, "x" * 63, 0)
        self.assertTrue(table.set_key(slot, "x" * 62, 0))

    def test_a_bad_index_is_an_error(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        for bad in (-1, 2**32, 1.5, "1"):
            with self.assertRaises((ValueError, TypeError), msg=repr(bad)):
                table.set_key(slot, "w", bad)  # type: ignore[arg-type]

    def test_the_shared_overflow_slot_cannot_be_named(self) -> None:
        table = SlotTable()
        for _ in range(OVERFLOW_SLOT):
            table.claim()
        slot, shared = table.claim()
        self.assertTrue(shared)
        self.assertFalse(table.set_key(slot, "late", 0))
        self.assertEqual(slot.key, "t#overflow")


class ViewTests(unittest.TestCase):
    def test_views_are_immutable_copies(self) -> None:
        table = SlotTable()
        slot, _ = table.claim()
        view = table.views()[0]
        slot.evals += 1
        slot.work += 5
        self.assertEqual((view.evals, view.work), (0, 0))
        with self.assertRaises(AttributeError):
            view.evals = 9  # type: ignore[misc]
        self.assertEqual(view.tape_consumed, 0)


if __name__ == "__main__":
    unittest.main()
