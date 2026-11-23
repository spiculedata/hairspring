//! VM names. The pinned module treats a virtual machine differently by its name, so every name this backend creates
//! has the one form the module's selector accepts: `vmvx_<8 digits>T<4 digits>Z_XB_r<2 digits>`. The same grammar
//! (with its other cell letters and the `smoke` suffix) is what the leftover scan reports.

/// The start of every name this backend creates and of every name the leftover scan reports.
pub const VM_NAME_PREFIX: &str = "vmvx_";
/// The cell letter of the names this backend creates.
const OWN_CELL: char = 'B';
/// Names per stamp: the two run digits.
pub const NAMES_PER_STAMP: u32 = 100;
const DATE_DIGITS: usize = 8;
const TIME_DIGITS: usize = 4;
const STAMP_LEN: usize = DATE_DIGITS + 1 + TIME_DIGITS + 1;

/// Why a name or stamp was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    BadStamp(String),
    RunOutOfRange(u32),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameError::BadStamp(s) => write!(f, "`{s}` is not a stamp of the form 12345678T1234Z"),
            NameError::RunOutOfRange(n) => {
                write!(f, "run number {n} is not from 1 to {}", NAMES_PER_STAMP - 1)
            }
        }
    }
}

impl std::error::Error for NameError {}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// A stamp: 8 digits, `T`, 4 digits, `Z`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp(String);

impl Stamp {
    pub fn parse(text: &str) -> Result<Stamp, NameError> {
        let ok = text.is_ascii()
            && text.len() == STAMP_LEN
            && all_digits(&text[..DATE_DIGITS])
            && text.as_bytes()[DATE_DIGITS] == b'T'
            && all_digits(&text[DATE_DIGITS + 1..DATE_DIGITS + 1 + TIME_DIGITS])
            && text.ends_with('Z');
        if ok {
            Ok(Stamp(text.to_string()))
        } else {
            Err(NameError::BadStamp(text.chars().take(32).collect()))
        }
    }

    /// The stamp for the `n`th allocation: the same grammar, not a time of day.
    pub fn from_counter(n: u64) -> Stamp {
        let (hi, lo) = (n / 10_000, n % 10_000);
        Stamp(format!("{:08}T{:04}Z", hi % 100_000_000, lo))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A name this backend creates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VmName(String);

impl VmName {
    pub fn new(stamp: &Stamp, run: u32) -> Result<VmName, NameError> {
        if run == 0 || run >= NAMES_PER_STAMP {
            return Err(NameError::RunOutOfRange(run));
        }
        Ok(VmName(format!(
            "{VM_NAME_PREFIX}{}_X{OWN_CELL}_r{run:02}",
            stamp.as_str()
        )))
    }

    /// Parses a name of exactly the form `new` builds.
    pub fn parse(text: &str) -> Option<VmName> {
        if !text.is_ascii() {
            return None;
        }
        let rest = text.strip_prefix(VM_NAME_PREFIX)?;
        let stamp = Stamp::parse(rest.get(..STAMP_LEN)?).ok()?;
        let digits = rest[STAMP_LEN..].strip_prefix("_XB_r")?;
        if digits.len() != 2 || !all_digits(digits) {
            return None;
        }
        VmName::new(&stamp, digits.parse().ok()?).ok()
    }

    /// The two run digits of the name.
    pub fn run_number(&self) -> u32 {
        self.0[self.0.len() - 2..].parse().unwrap_or(0)
    }

    /// The stamp in the name.
    pub fn stamp(&self) -> Stamp {
        let start = VM_NAME_PREFIX.len();
        Stamp(self.0[start..start + STAMP_LEN].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VmName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `text` is a name of the module's selector grammar: `vmvx_<stamp>_X<B|C|N>_(r<2 digits>|smoke)`.
pub fn is_selector_name(text: &str) -> bool {
    if !text.is_ascii() {
        return false;
    }
    let Some(rest) = text.strip_prefix(VM_NAME_PREFIX) else {
        return false;
    };
    if rest.len() < STAMP_LEN || Stamp::parse(&rest[..STAMP_LEN]).is_err() {
        return false;
    }
    let Some(tail) = rest[STAMP_LEN..].strip_prefix("_X") else {
        return false;
    };
    let mut chars = tail.chars();
    let cell_ok = matches!(chars.next(), Some('B' | 'C' | 'N'));
    let run = chars.as_str().strip_prefix('_');
    cell_ok
        && run.is_some_and(|r| {
            r == "smoke" || (r.len() == 3 && r.starts_with('r') && all_digits(&r[1..]))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_has_the_selector_form_and_26_bytes() {
        let stamp = Stamp::parse("20261123T1530Z").unwrap();
        let name = VmName::new(&stamp, 7).unwrap();
        assert_eq!(name.as_str(), "vmvx_20261123T1530Z_XB_r07");
        assert_eq!(name.as_str().len(), 26);
        assert!(is_selector_name(name.as_str()));
        assert_eq!(VmName::parse(name.as_str()), Some(name));
    }

    #[test]
    fn a_run_number_of_zero_or_of_100_or_more_is_refused() {
        let stamp = Stamp::from_counter(1);
        assert!(VmName::new(&stamp, 1).is_ok());
        assert!(VmName::new(&stamp, 99).is_ok());
        assert_eq!(VmName::new(&stamp, 0), Err(NameError::RunOutOfRange(0)));
        assert_eq!(VmName::new(&stamp, 100), Err(NameError::RunOutOfRange(100)));
        let name = VmName::new(&stamp, 16).unwrap();
        assert_eq!((name.run_number(), name.stamp()), (16, stamp));
    }

    #[test]
    fn stamps_from_a_counter_are_distinct_and_in_the_grammar() {
        let a = Stamp::from_counter(1);
        let b = Stamp::from_counter(2);
        let c = Stamp::from_counter(10_001);
        assert_ne!(a, b);
        assert_ne!(b, c);
        for s in [&a, &b, &c] {
            assert_eq!(Stamp::parse(s.as_str()).as_ref(), Ok(s));
        }
    }

    #[test]
    fn bad_stamps_are_refused() {
        for bad in [
            "",
            "20261123T153Z",
            "20261123T1530",
            "2026112aT1530Z",
            "20261123t1530Z",
            "20261123T1530Z ",
            "2026112\u{e9}T1530Z",
        ] {
            assert!(Stamp::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_selector_grammar_takes_the_three_cells_and_the_two_suffixes() {
        for ok in [
            "vmvx_20261123T1530Z_XB_r04",
            "vmvx_20261123T1530Z_XC_r99",
            "vmvx_20261123T1530Z_XN_smoke",
        ] {
            assert!(is_selector_name(ok), "{ok}");
        }
        for bad in [
            "hs_20261123T1530Z_XB_r04",
            "vmvx_20261123T1530Z_XD_r04",
            "vmvx_20261123T1530Z_XB_r4",
            "vmvx_20261123T1530Z_XB_r004",
            "vmvx_20261123T1530Z_XB_smoke2",
            "vmvx_20261123T1530Z_XB",
            "vmvx_20261123T1530Z_XB_r0\u{1b}",
            "vmvx_",
            "",
        ] {
            assert!(!is_selector_name(bad), "{bad}");
        }
    }

    #[test]
    fn only_the_own_cell_form_parses_as_a_created_name() {
        assert!(VmName::parse("vmvx_20261123T1530Z_XC_r04").is_none());
        assert!(VmName::parse("vmvx_20261123T1530Z_XB_smoke").is_none());
        assert!(VmName::parse("vmvx_20261123T1530Z_XB_r04x").is_none());
        assert!(VmName::parse("other").is_none());
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(NameError::BadStamp("x".into())
            .to_string()
            .contains("stamp"));
        assert!(NameError::RunOutOfRange(100).to_string().contains("100"));
    }
}
