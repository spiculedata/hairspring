import unittest

from hairspring import _fmt


class NumberFieldTests(unittest.TestCase):
    def test_counters_are_right_aligned_in_twenty_characters(self) -> None:
        self.assertEqual(_fmt.num_w(5), " " * 19 + "5")
        self.assertEqual(_fmt.num_w(2**64 - 1), "18446744073709551615")
        self.assertEqual(len(_fmt.num_w(0)), 20)

    def test_signed_counters_keep_the_sign_inside_the_field(self) -> None:
        self.assertEqual(_fmt.i64_w(-7), " " * 18 + "-7")
        self.assertEqual(len(_fmt.i64_w(-(2**63))), 20)
        self.assertTrue(_fmt.i64_w(-(2**63)).endswith("-9223372036854775808"))
        self.assertEqual(_fmt.i64_w(12), " " * 18 + "12")

    def test_hex_is_sixteen_lowercase_digits(self) -> None:
        self.assertEqual(_fmt.hex64(0xFB6CF21A75AAD6EF), "fb6cf21a75aad6ef")
        self.assertEqual(_fmt.hex64(0xAB), "00000000000000ab")


class TextTests(unittest.TestCase):
    def test_free_text_escapes_quotes_and_backslashes_and_masks_other_bytes(self) -> None:
        # U+00E9 is two bytes in UTF-8, so it becomes two question marks; a newline is one.
        self.assertEqual(_fmt.json_text('a"b\\cé\n', _fmt.MSG_MAX), '"a\\"b\\\\c???"')

    def test_free_text_is_cut_at_the_limit_in_bytes(self) -> None:
        self.assertEqual(_fmt.json_text("abcdef", 3), '"abc"')
        self.assertEqual(_fmt.json_text("éé", 3), '"???"')

    def test_ids_replace_bytes_outside_the_alphabet(self) -> None:
        self.assertEqual(_fmt.ident("a b#c.d:e/f-g_h", _fmt.ID_MAX), '"a_b_c.d:e/f-g_h"')

    def test_sanitize_keeps_the_sent_form_and_the_limit(self) -> None:
        self.assertEqual(_fmt.sanitize_ident("a b", _fmt.ID_MAX), "a_b")
        self.assertEqual(_fmt.sanitize_ident("abcdef", 3), "abc")
        self.assertEqual(_fmt.sanitize_ident("café", _fmt.ID_MAX), "caf__")

    def test_thread_keys_keep_the_hash_and_pad_to_sixty_four_bytes(self) -> None:
        self.assertEqual(_fmt.thread_key_padded("t#1"), '"t#1"' + " " * 61)
        self.assertEqual(_fmt.thread_key("a b#c", _fmt.THREAD_KEY_MAX), '"a_b#c"')

    def test_alphabet(self) -> None:
        for ch in "azAZ09._:/-":
            self.assertTrue(_fmt.is_ident_char(ord(ch)), ch)
        for ch in "# $\"\\":
            self.assertFalse(_fmt.is_ident_char(ord(ch)), ch)


class HashTests(unittest.TestCase):
    def test_the_hash_uses_the_sent_form_and_the_limit(self) -> None:
        self.assertEqual(_fmt.ident_hash("a b", _fmt.ID_MAX), _fmt.ident_hash("a_b", _fmt.ID_MAX))
        self.assertNotEqual(_fmt.ident_hash("a", _fmt.ID_MAX), _fmt.ident_hash("ab", _fmt.ID_MAX))
        self.assertEqual(_fmt.ident_hash("abcX", 3), _fmt.ident_hash("abcY", 3))

    def test_the_hash_is_fnv_1a_64(self) -> None:
        h = 0xCBF29CE484222325
        for c in b"basic.always.ok":
            h = ((h ^ c) * 0x00000100000001B3) & (2**64 - 1)
        self.assertEqual(_fmt.ident_hash("basic.always.ok", _fmt.ID_MAX), h)
        self.assertEqual(_fmt.fnv1a64(b""), 0xCBF29CE484222325)


if __name__ == "__main__":
    unittest.main()
