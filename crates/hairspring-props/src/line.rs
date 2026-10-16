//! Parsing of one guest output line into a typed event.

use crate::decode::{decode_event, Decoded};
use crate::event::Event;
use crate::json::{parse_strict, JsonError};
use crate::limits::{EVENT_PREFIX, EVENT_PREFIX_BARE, MAX_LINE_BYTES};

/// Why a line that claimed to be an event was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    TooLong,
    InvalidUtf8,
    MissingSeparator,
    MalformedJson,
    DuplicateKey,
    TooDeep,
    NotAnObject,
    MissingField(&'static str),
    InvalidField(&'static str),
    OverLimit(&'static str),
    Inconsistent(&'static str),
}

/// Why a well-formed line was set aside without error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    UnknownKind,
    UnsupportedVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOutcome {
    /// Ordinary program output.
    NotEvent,
    Event {
        event: Event,
        /// Fields the parser did not recognise (counted, otherwise ignored).
        unknown_fields: u32,
    },
    Ignored(IgnoreReason),
    Rejected(RejectReason),
}

/// Whether the line claims to be an event, well formed or not. Ordinary output that merely begins with the same
/// letters (for example `HAIRSPRING_EVENTS_FILE=...`) is not a candidate.
pub(crate) fn is_event_candidate(line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.starts_with(EVENT_PREFIX.as_bytes()) {
        return true;
    }
    line.starts_with(EVENT_PREFIX_BARE.as_bytes())
        && matches!(
            line.get(EVENT_PREFIX_BARE.len()),
            None | Some(b'{') | Some(b'\t')
        )
}

/// Parse one line (newline already removed; a trailing `\r` is tolerated). Never panics.
pub fn parse_line(line: &[u8]) -> LineOutcome {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let prefix = EVENT_PREFIX.as_bytes();
    if !line.starts_with(EVENT_PREFIX_BARE.as_bytes()) {
        return LineOutcome::NotEvent;
    }
    if !line.starts_with(prefix) {
        // "HAIRSPRING_EVENT" alone or glued to a JSON object is a broken event; any other continuation
        // (for example "HAIRSPRING_EVENTS_FILE=...") is ordinary output.
        return match line.get(EVENT_PREFIX_BARE.len()) {
            None | Some(b'{') | Some(b'\t') => {
                LineOutcome::Rejected(RejectReason::MissingSeparator)
            }
            Some(_) => LineOutcome::NotEvent,
        };
    }
    if line.len() > MAX_LINE_BYTES {
        return LineOutcome::Rejected(RejectReason::TooLong);
    }
    let Ok(body) = std::str::from_utf8(&line[prefix.len()..]) else {
        return LineOutcome::Rejected(RejectReason::InvalidUtf8);
    };
    let root = match parse_strict(body) {
        Ok(root) => root,
        Err(JsonError::Malformed) => return LineOutcome::Rejected(RejectReason::MalformedJson),
        Err(JsonError::DuplicateKey) => return LineOutcome::Rejected(RejectReason::DuplicateKey),
        Err(JsonError::TooDeep) => return LineOutcome::Rejected(RejectReason::TooDeep),
    };
    match decode_event(&root) {
        Ok(Decoded::Event {
            event,
            unknown_fields,
        }) => LineOutcome::Event {
            event: *event,
            unknown_fields,
        },
        Ok(Decoded::Ignored(reason)) => LineOutcome::Ignored(reason),
        Err(reason) => LineOutcome::Rejected(reason),
    }
}
