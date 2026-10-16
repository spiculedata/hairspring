use super::*;

const RUN: &str = "run_0123456789abcdef";

fn addr(text: &str) -> Result<PointAddress, AddressError> {
    PointAddress::parse(text)
}

#[test]
fn parses_each_cut_kind() {
    let end = addr("p1~run_0123456789abcdef~7~end").unwrap();
    assert_eq!((end.run_id(), end.seq(), end.cut()), (RUN, 7, Cut::End));
    assert_eq!(
        addr("p1~run_0123456789abcdef~0~s12").unwrap().cut(),
        Cut::Step(12)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~0~c0").unwrap().cut(),
        Cut::Choice(0)
    );
}

#[test]
fn formats_the_documented_shape() {
    let a = PointAddress::new(RUN, 42, Cut::Step(3)).unwrap();
    assert_eq!(a.to_string(), "p1~run_0123456789abcdef~42~s3");
    let b = PointAddress::new(RUN, 1, Cut::End).unwrap();
    assert_eq!(b.to_string(), "p1~run_0123456789abcdef~1~end");
}

#[test]
fn accepts_the_largest_numbers_and_stays_within_the_length_bound() {
    let text = "p1~run_0123456789abcdef~9999999999999999999~c9999999999999999999";
    let a = addr(text).unwrap();
    assert_eq!(a.seq(), 9_999_999_999_999_999_999);
    assert_eq!(a.to_string(), text);
    assert_eq!(text.len(), MAX_ADDRESS_BYTES);
}

#[test]
fn rejects_unknown_versions_and_cut_kinds() {
    assert_eq!(
        addr("p2~run_0123456789abcdef~1~end"),
        Err(AddressError::BadVersion)
    );
    assert_eq!(
        addr("P1~run_0123456789abcdef~1~end"),
        Err(AddressError::BadVersion)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~t4"),
        Err(AddressError::UnknownCutKind)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~End"),
        Err(AddressError::UnknownCutKind)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~ends"),
        Err(AddressError::UnknownCutKind)
    );
}

#[test]
fn rejects_leading_zeros_signs_and_non_digits() {
    for bad in ["01", "00", "+1", "-1", "1 ", " 1", "1.0", "0x1", "\u{0661}"] {
        let text = format!("p1~{RUN}~{bad}~end");
        assert_eq!(addr(&text), Err(AddressError::BadNumber), "seq {bad:?}");
    }
    for bad in ["s01", "c007", "s+1", "s-1", "sx", "c1e3"] {
        let text = format!("p1~{RUN}~1~{bad}");
        assert_eq!(addr(&text), Err(AddressError::BadNumber), "cut {bad:?}");
    }
}

#[test]
fn rejects_twenty_digit_numbers_even_when_they_fit() {
    let overflow = "p1~run_0123456789abcdef~18446744073709551616~end";
    assert_eq!(addr(overflow), Err(AddressError::BadNumber));
    let twenty = "p1~run_0123456789abcdef~10000000000000000000~end";
    assert_eq!(addr(twenty), Err(AddressError::BadNumber));
    let cut = "p1~run_0123456789abcdef~1~s10000000000000000000";
    assert_eq!(addr(cut), Err(AddressError::BadNumber));
}

#[test]
fn rejects_empty_parts() {
    assert_eq!(addr(""), Err(AddressError::WrongPartCount));
    assert_eq!(addr("p1~~1~end"), Err(AddressError::EmptyPart));
    assert_eq!(
        addr("~run_0123456789abcdef~1~end"),
        Err(AddressError::EmptyPart)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~~end"),
        Err(AddressError::EmptyPart)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~"),
        Err(AddressError::EmptyPart)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~s"),
        Err(AddressError::BadNumber)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~c"),
        Err(AddressError::BadNumber)
    );
}

#[test]
fn rejects_trailing_junk_and_tilde_injection() {
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~end~"),
        Err(AddressError::WrongPartCount)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~end~x"),
        Err(AddressError::WrongPartCount)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1"),
        Err(AddressError::WrongPartCount)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~end "),
        Err(AddressError::UnknownCutKind)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~end\n"),
        Err(AddressError::UnknownCutKind)
    );
    assert_eq!(
        addr("p1~run_0123456789abcdef~1~end%7Ex"),
        Err(AddressError::UnknownCutKind)
    );
}

#[test]
fn rejects_malformed_run_ids() {
    for bad in [
        "run_0123456789abcde",
        "run_0123456789abcdef0",
        "run_0123456789ABCDEF",
        "run_0123456789abcdeg",
        "job_0123456789abcdef",
        "0123456789abcdef",
        "run_0123456789abcde\u{e9}",
    ] {
        let text = format!("p1~{bad}~1~end");
        assert_eq!(addr(&text), Err(AddressError::BadRunId), "run id {bad:?}");
    }
}

#[test]
fn enforces_the_length_bound_in_bytes() {
    let ok = format!("p1~{RUN}~1~end");
    assert!(ok.len() <= MAX_ADDRESS_BYTES);
    let long = format!("p1~{RUN}~1~end{}", "x".repeat(MAX_ADDRESS_BYTES));
    assert_eq!(addr(&long), Err(AddressError::TooLong));
    let sixty_five = "p1~run_0123456789abcdef~9999999999999999999~c99999999999999999990";
    assert_eq!(sixty_five.len(), MAX_ADDRESS_BYTES + 1);
    assert_eq!(addr(sixty_five), Err(AddressError::TooLong));
    assert_eq!(
        addr(&"p".repeat(MAX_ADDRESS_BYTES + 1)),
        Err(AddressError::TooLong)
    );
    assert_eq!(
        addr(&"p".repeat(MAX_ADDRESS_BYTES)),
        Err(AddressError::WrongPartCount)
    );
    // Multi-byte characters count as bytes, not characters.
    let wide = "\u{e9}".repeat(MAX_ADDRESS_BYTES / 2 + 1);
    assert_eq!(addr(&wide), Err(AddressError::TooLong));
}

#[test]
fn cut_kind_must_match_the_backend() {
    let step = format!("p1~{RUN}~1~s4");
    let choice = format!("p1~{RUN}~1~c4");
    let end = format!("p1~{RUN}~1~end");
    use BackendKind::*;
    assert!(PointAddress::parse_for_backend(&step, Sim).is_ok());
    assert!(PointAddress::parse_for_backend(&choice, Wasm).is_ok());
    for backend in [Sim, Wasm, Hermit] {
        assert!(PointAddress::parse_for_backend(&end, backend).is_ok());
    }
    let bad = Err(AddressError::CutNotValidForBackend);
    assert_eq!(PointAddress::parse_for_backend(&step, Wasm), bad);
    assert_eq!(PointAddress::parse_for_backend(&choice, Sim), bad);
    assert_eq!(PointAddress::parse_for_backend(&step, Hermit), bad);
    assert_eq!(PointAddress::parse_for_backend(&choice, Hermit), bad);
}

#[test]
fn new_validates_the_run_id() {
    assert_eq!(
        PointAddress::new("run_short", 1, Cut::End),
        Err(AddressError::BadRunId)
    );
    assert_eq!(
        PointAddress::new("run_0123456789abcdef~", 1, Cut::End),
        Err(AddressError::BadRunId)
    );
}

/// Deterministic corpus: every cut kind against a spread of boundary and pseudo-random numbers.
fn corpus() -> Vec<PointAddress> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let boundaries = [
        0,
        1,
        9,
        10,
        99,
        100,
        u64::MAX / 2,
        9_999_999_999_999_999_999,
    ];
    let mut numbers: Vec<u64> = boundaries.to_vec();
    for _ in 0..200 {
        // Shift to get every digit count from 1 to 19.
        let shifted = next() >> (next() % 64);
        numbers.push(shifted.min(9_999_999_999_999_999_999));
    }
    let mut out = Vec::new();
    for (i, n) in numbers.iter().enumerate() {
        let run = format!("run_{:016x}", next());
        let cut = match i % 3 {
            0 => Cut::End,
            1 => Cut::Step(*n),
            _ => Cut::Choice(*n),
        };
        out.push(PointAddress::new(&run, numbers[(i + 1) % numbers.len()], cut).unwrap());
    }
    out
}

#[test]
fn format_then_parse_round_trips_over_a_generated_corpus() {
    for a in corpus() {
        let text = a.to_string();
        assert!(text.len() <= MAX_ADDRESS_BYTES, "{text}");
        assert_eq!(PointAddress::parse(&text), Ok(a.clone()), "{text}");
    }
}

#[test]
fn parse_then_format_is_the_identity_for_accepted_text() {
    for a in corpus() {
        let text = a.to_string();
        assert_eq!(PointAddress::parse(&text).unwrap().to_string(), text);
    }
}

#[test]
fn no_single_byte_mutation_of_a_valid_address_changes_it_silently() {
    // Whatever a one-byte edit does, an accepted result must format back to the edited text.
    let base = format!("p1~{RUN}~12~s34");
    for i in 0..base.len() {
        for replacement in [b'0', b'~', b' ', b'a', b'Z', b'-', 0x7f] {
            let mut bytes = base.clone().into_bytes();
            bytes[i] = replacement;
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            if let Ok(parsed) = PointAddress::parse(&text) {
                assert_eq!(
                    parsed.to_string(),
                    text,
                    "accepted but not canonical: {text:?}"
                );
            }
        }
    }
}
