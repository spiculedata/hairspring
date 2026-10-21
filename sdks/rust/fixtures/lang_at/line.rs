//! The one stdout line and the hash of the arrival order.

use super::mode::Mode;

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// The hash of the arrival list (worker numbers 0 to 3) written as ASCII digits.
pub fn order_hash(arrival: &[u64]) -> u64 {
    let digits: Vec<u8> = arrival.iter().map(|&w| b'0' + (w % 10) as u8).collect();
    fnv1a64(&digits)
}

/// `LANGAT v1 lang=rust mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>` and a newline.
pub fn format_line(mode: Mode, final_value: u64, expect: u64, order: u64, rand: u64) -> String {
    format!(
        "LANGAT v1 lang=rust mode={} final={final_value:020} expect={expect:020} order={order:016x} rand={rand:016x}\n",
        mode.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_the_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn the_order_hash_is_the_hash_of_four_ascii_digits() {
        assert_eq!(order_hash(&[2, 0, 1, 3]), fnv1a64(b"2013"));
        assert_ne!(order_hash(&[0, 1, 2, 3]), order_hash(&[3, 2, 1, 0]));
    }

    #[test]
    fn the_line_has_the_contract_form() {
        let line = format_line(Mode::Buggy, 3_998, 4_000, 0xab, 0x1234);
        assert_eq!(
            line,
            "LANGAT v1 lang=rust mode=buggy final=00000000000000003998 \
             expect=00000000000000004000 order=00000000000000ab rand=0000000000001234\n"
        );
    }

    #[test]
    fn the_line_names_each_mode() {
        for (mode, name) in [
            (Mode::Buggy, "buggy"),
            (Mode::Fixed, "fixed"),
            (Mode::Wide, "wide"),
        ] {
            assert!(format_line(mode, 0, 0, 0, 0).contains(&format!(" mode={name} ")));
        }
    }
}
