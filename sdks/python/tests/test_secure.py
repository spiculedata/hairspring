import struct
import sys
import unittest

from hairspring import _secure as secure

AT_NULL = 0
AT_SECURE = 23
AT_PAGESZ = 6


def auxv(*pairs: tuple[int, int]) -> bytes:
    word = "Q" if struct.calcsize("P") == 8 else "I"
    return b"".join(struct.pack("=" + word * 2, kind, value) for kind, value in pairs)


def failing() -> bytes:
    raise PermissionError("not dumpable")


class AtSecureTests(unittest.TestCase):
    def test_a_set_flag_is_privileged(self) -> None:
        raw = auxv((AT_PAGESZ, 4096), (AT_SECURE, 1), (AT_NULL, 0))
        self.assertEqual(secure.at_secure_from(raw), (True, True))

    def test_a_clear_flag_is_not_privileged(self) -> None:
        raw = auxv((AT_SECURE, 0), (AT_NULL, 0))
        self.assertEqual(secure.at_secure_from(raw), (True, False))

    def test_no_entry_is_reported_as_not_found(self) -> None:
        raw = auxv((AT_PAGESZ, 4096), (AT_NULL, 0), (AT_SECURE, 1))
        self.assertEqual(secure.at_secure_from(raw), (False, False))

    def test_a_truncated_vector_is_not_found(self) -> None:
        self.assertEqual(secure.at_secure_from(b"\x01\x02"), (False, False))


class DecisionTests(unittest.TestCase):
    def test_the_vector_decides_when_it_has_the_entry(self) -> None:
        raw = auxv((AT_SECURE, 1), (AT_NULL, 0))
        self.assertTrue(secure.is_privileged_with(lambda: raw, lambda: False, linux=True))
        raw = auxv((AT_SECURE, 0), (AT_NULL, 0))
        self.assertFalse(secure.is_privileged_with(lambda: raw, lambda: True, linux=True))

    def test_an_unreadable_vector_on_linux_means_privileged(self) -> None:
        # A file-capability binary that is not dumpable cannot read its own vector: the case AT_SECURE exists for.
        self.assertTrue(secure.is_privileged_with(failing, lambda: False, linux=True))

    def test_without_an_entry_or_off_linux_the_ids_decide(self) -> None:
        raw = auxv((AT_NULL, 0))
        self.assertTrue(secure.is_privileged_with(lambda: raw, lambda: True, linux=True))
        self.assertFalse(secure.is_privileged_with(lambda: raw, lambda: False, linux=True))
        self.assertTrue(secure.is_privileged_with(failing, lambda: True, linux=False))
        self.assertFalse(secure.is_privileged_with(failing, lambda: False, linux=False))


class ThisProcessTests(unittest.TestCase):
    def test_an_ordinary_test_process_is_not_privileged(self) -> None:
        self.assertFalse(secure.is_privileged())

    def test_the_real_vector_is_readable_on_linux(self) -> None:
        if not sys.platform.startswith("linux"):
            self.skipTest("Linux only")
        found, secure_flag = secure.at_secure_from(secure.read_auxv())
        self.assertTrue(found)
        self.assertFalse(secure_flag)


if __name__ == "__main__":
    unittest.main()
