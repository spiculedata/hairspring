import unittest

from hairspring import _identity as ident
from hairspring._identity import Identity


def ident_of(tag: str, stable: bool, refused: bool) -> Identity:
    return Identity(tag=tag, tag_stable=stable, refused=refused)


class ResolveTests(unittest.TestCase):
    def test_without_a_harness_tag_the_root_process_is_p1(self) -> None:
        self.assertEqual(ident.resolve(None, None, 10), ident_of("p1", True, False))
        self.assertEqual(ident.resolve("", None, 10), ident_of("p1", True, False))

    def test_a_harness_tag_without_an_owner_is_the_root_tag(self) -> None:
        self.assertEqual(ident.resolve("job-7", None, 10), ident_of("job-7", True, False))

    def test_an_invalid_harness_tag_is_refused_not_shortened(self) -> None:
        for bad in ("_eval", "a b", "x" * 65):
            self.assertEqual(ident.resolve(bad, None, 10), ident_of("p1", True, True), bad)

    def test_tag_validity_at_its_limits(self) -> None:
        self.assertTrue(ident.tag_valid("a"))
        self.assertTrue(ident.tag_valid("x" * 64))
        self.assertFalse(ident.tag_valid(""))
        self.assertFalse(ident.tag_valid("_eval"))
        self.assertFalse(ident.tag_valid("a#b"))

    def test_a_child_whose_inherited_pid_is_not_its_own_takes_a_spawn_tag(self) -> None:
        self.assertEqual(ident.resolve("p1", "999", 42), ident_of("p1.s42", False, False))
        self.assertEqual(ident.resolve("p1", "junk", 42), ident_of("p1.s42", False, False))

    def test_an_image_that_owns_the_inherited_pid_keeps_the_tag(self) -> None:
        self.assertEqual(ident.resolve("p1", "42", 42), ident_of("p1", True, False))

    def test_an_exec_without_fork_after_the_report_takes_an_x_tag(self) -> None:
        self.assertEqual(ident.resolve("p1", "42:a", 42), ident_of("p1.x1", True, False))

    def test_an_unstable_tag_stays_unstable_across_an_exec(self) -> None:
        self.assertEqual(ident.resolve("p1.s9", "42:u", 42), ident_of("p1.s9", False, False))
        self.assertEqual(ident.resolve("p1.s9", "42:au", 42), ident_of("p1.s9.x1", False, False))


class OwnerFieldTests(unittest.TestCase):
    def test_the_owner_field_records_the_pid_and_its_flags(self) -> None:
        self.assertEqual(ident.owner_field(42, False, True), "42")
        self.assertEqual(ident.owner_field(42, True, True), "42:a")
        self.assertEqual(ident.owner_field(42, False, False), "42:u")
        self.assertEqual(ident.owner_field(42, True, False), "42:au")


class DeriveTests(unittest.TestCase):
    def test_derived_tags_append_a_letter_and_a_number(self) -> None:
        self.assertEqual(ident.derive("p1", "f", 2), "p1.f2")

    def test_a_derived_tag_that_would_pass_64_bytes_keeps_a_prefix_and_a_hash(self) -> None:
        base = "b" * 62
        tag = ident.derive(base, "s", 123456)
        self.assertLessEqual(len(tag), 64, tag)
        self.assertTrue(tag.startswith("b" * 40))
        self.assertIn(".h", tag)
        self.assertTrue(ident.tag_valid(tag))
        self.assertNotEqual(tag, ident.derive(base, "s", 123457))


if __name__ == "__main__":
    unittest.main()
