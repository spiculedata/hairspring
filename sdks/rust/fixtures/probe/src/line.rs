//! The one stdout line and the hashes in it.

/// The six values of the line, in the order they are printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    pub lock_order: u64,
    pub arrival: u64,
    pub rand: u64,
    pub vtime_ns: u64,
    pub extra: u64,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// The hash of a sequence: each item as decimal text followed by a comma, so item boundaries count.
pub fn hash_sequence(items: &[u64]) -> u64 {
    let text: String = items.iter().map(|item| format!("{item},")).collect();
    fnv1a64(text.as_bytes())
}

/// `PROBE v1 lang=rust lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>` and a
/// newline.
pub fn format_line(fields: &Fields) -> String {
    format!(
        "PROBE v1 lang=rust lock_order={:016x} arrival={:016x} rand={:016x} vtime_ns={:020} extra={:016x}\n",
        fields.lock_order, fields.arrival, fields.rand, fields.vtime_ns, fields.extra
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
    fn a_sequence_hashes_as_its_decimal_items_each_followed_by_a_comma() {
        assert_eq!(hash_sequence(&[2, 0, 1, 3]), fnv1a64(b"2,0,1,3,"));
        assert_eq!(hash_sequence(&[10, 1]), fnv1a64(b"10,1,"));
        assert_eq!(hash_sequence(&[]), fnv1a64(b""));
    }

    #[test]
    fn the_order_of_a_sequence_matters_and_its_item_boundaries_do_too() {
        assert_ne!(hash_sequence(&[0, 1, 2, 3]), hash_sequence(&[3, 2, 1, 0]));
        assert_ne!(hash_sequence(&[1, 11]), hash_sequence(&[11, 1]));
        assert_ne!(hash_sequence(&[1, 11]), hash_sequence(&[111]));
    }

    #[test]
    fn the_line_has_the_contract_form() {
        let fields = Fields {
            lock_order: 0xab,
            arrival: 0xcd,
            rand: 0x1234,
            vtime_ns: 5_000_000,
            extra: 0xffff_ffff_ffff_ffff,
        };
        assert_eq!(
            format_line(&fields),
            "PROBE v1 lang=rust lock_order=00000000000000ab arrival=00000000000000cd \
             rand=0000000000001234 vtime_ns=00000000000005000000 extra=ffffffffffffffff\n"
        );
    }

    #[test]
    fn the_largest_virtual_time_still_has_twenty_digits() {
        let fields = Fields {
            lock_order: 0,
            arrival: 0,
            rand: 0,
            vtime_ns: u64::MAX,
            extra: 0,
        };
        assert!(format_line(&fields).contains(" vtime_ns=18446744073709551615 "));
    }

    #[test]
    fn the_same_fields_always_give_the_same_line() {
        let fields = Fields {
            lock_order: 1,
            arrival: 2,
            rand: 3,
            vtime_ns: 4,
            extra: 5,
        };
        assert_eq!(format_line(&fields), format_line(&fields));
    }
}
