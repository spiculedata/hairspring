//! The point address: `p1~<run_id>~<seq>~<cut>`.
//!
//! Parsing is strict and total: every accepted address formats back to exactly the same text.
use std::fmt;

/// Version tag that starts every address.
pub const ADDRESS_VERSION: &str = "p1";
/// Longest accepted address, in bytes.
pub const MAX_ADDRESS_BYTES: usize = 64;
/// Most digits accepted in a number (every 19-digit number fits in a `u64`).
pub const MAX_NUMBER_DIGITS: usize = 19;

const SEPARATOR: char = '~';
const PART_COUNT: usize = 4;
const RUN_ID_PREFIX: &str = "run_";
const RUN_ID_HEX_DIGITS: usize = 16;
const CUT_END: &str = "end";
const CUT_STEP: char = 's';
const CUT_CHOICE: char = 'c';

/// The kind of backend that produced a run; decides which cuts are valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Sim,
    Wasm,
    Hermit,
}

impl BackendKind {
    /// The backend name used in identity text and bundles.
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Sim => "sim",
            BackendKind::Wasm => "wasm",
            BackendKind::Hermit => "hermit",
        }
    }
}

/// Where in a trial a point sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// The whole trial.
    End,
    /// After `k` steps from the root (sim).
    Step(u64),
    /// After `k` choices (WASM).
    Choice(u64),
}

impl Cut {
    fn is_valid_for(self, backend: BackendKind) -> bool {
        matches!(
            (self, backend),
            (Cut::End, _) | (Cut::Step(_), BackendKind::Sim) | (Cut::Choice(_), BackendKind::Wasm)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressError {
    TooLong,
    EmptyPart,
    WrongPartCount,
    BadVersion,
    BadRunId,
    BadNumber,
    UnknownCutKind,
    CutNotValidForBackend,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            AddressError::TooLong => "point address is longer than 64 bytes",
            AddressError::EmptyPart => "point address has an empty part",
            AddressError::WrongPartCount => {
                "point address must have exactly four `~` separated parts"
            }
            AddressError::BadVersion => "point address version must be `p1`",
            AddressError::BadRunId => {
                "point address run id must be `run_` and 16 lowercase hex digits"
            }
            AddressError::BadNumber => {
                "point address number must be 1 to 19 digits with no leading zero"
            }
            AddressError::UnknownCutKind => "point address cut must be `end`, `s<k>` or `c<k>`",
            AddressError::CutNotValidForBackend => {
                "this cut kind is not valid for the run's backend"
            }
        };
        f.write_str(text)
    }
}

impl std::error::Error for AddressError {}

/// A parsed point address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointAddress {
    run_id: String,
    seq: u64,
    cut: Cut,
}

impl PointAddress {
    /// Builds an address, checking the run id so every value formats to parseable text.
    pub fn new(run_id: &str, seq: u64, cut: Cut) -> Result<Self, AddressError> {
        if !is_run_id(run_id) {
            return Err(AddressError::BadRunId);
        }
        Ok(PointAddress {
            run_id: run_id.to_string(),
            seq,
            cut,
        })
    }

    /// Parses an address without knowing the backend. Use [`Self::parse_for_backend`] when the
    /// run is known.
    pub fn parse(text: &str) -> Result<Self, AddressError> {
        if text.len() > MAX_ADDRESS_BYTES {
            return Err(AddressError::TooLong);
        }
        let parts: Vec<&str> = text.split(SEPARATOR).collect();
        if parts.len() != PART_COUNT {
            return Err(AddressError::WrongPartCount);
        }
        if parts.iter().any(|p| p.is_empty()) {
            return Err(AddressError::EmptyPart);
        }
        if parts[0] != ADDRESS_VERSION {
            return Err(AddressError::BadVersion);
        }
        let seq = parse_number(parts[2])?;
        let cut = parse_cut(parts[3])?;
        PointAddress::new(parts[1], seq, cut)
    }

    /// Parses an address and checks the cut kind against the run's backend.
    pub fn parse_for_backend(text: &str, backend: BackendKind) -> Result<Self, AddressError> {
        let address = Self::parse(text)?;
        address.check_backend(backend)?;
        Ok(address)
    }

    /// `end` is valid everywhere; `s<k>` only for sim; `c<k>` only for WASM.
    pub fn check_backend(&self, backend: BackendKind) -> Result<(), AddressError> {
        if self.cut.is_valid_for(backend) {
            Ok(())
        } else {
            Err(AddressError::CutNotValidForBackend)
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn cut(&self) -> Cut {
        self.cut
    }
}

fn is_run_id(text: &str) -> bool {
    let Some(hex) = text.strip_prefix(RUN_ID_PREFIX) else {
        return false;
    };
    hex.len() == RUN_ID_HEX_DIGITS
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A decimal number of 1 to 19 ASCII digits with no leading zero (a lone `0` is allowed).
fn parse_number(text: &str) -> Result<u64, AddressError> {
    let bytes = text.as_bytes();
    let well_formed = !bytes.is_empty()
        && bytes.len() <= MAX_NUMBER_DIGITS
        && bytes.iter().all(u8::is_ascii_digit)
        && (bytes.len() == 1 || bytes[0] != b'0');
    if !well_formed {
        return Err(AddressError::BadNumber);
    }
    text.parse().map_err(|_| AddressError::BadNumber)
}

fn parse_cut(text: &str) -> Result<Cut, AddressError> {
    if text == CUT_END {
        return Ok(Cut::End);
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(CUT_STEP) => parse_number(chars.as_str()).map(Cut::Step),
        Some(CUT_CHOICE) => parse_number(chars.as_str()).map(Cut::Choice),
        _ => Err(AddressError::UnknownCutKind),
    }
}

impl fmt::Display for Cut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cut::End => f.write_str(CUT_END),
            Cut::Step(k) => write!(f, "{CUT_STEP}{k}"),
            Cut::Choice(k) => write!(f, "{CUT_CHOICE}{k}"),
        }
    }
}

impl fmt::Display for PointAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{ADDRESS_VERSION}{SEPARATOR}{}{SEPARATOR}{}{SEPARATOR}{}",
            self.run_id, self.seq, self.cut
        )
    }
}

#[cfg(test)]
mod tests;
