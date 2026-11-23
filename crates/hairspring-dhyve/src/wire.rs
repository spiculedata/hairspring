//! The control channel's messages: fixed 72-byte little-endian records, `<IHHQQQQQQQII>`: magic, version, type, seven
//! 64-bit fields and two 32-bit ones. The harness that drove the module before this crate is the reference for the
//! layout; the module's source was not re-read.

/// Size of every message.
pub const SIZE: usize = 72;
const MAGIC: u32 = 0x4452_5631;
const VERSION: u16 = 1;
/// The branch count a RESUME sets as the fence. Zero would disarm the fence, so the largest accepted value is used.
pub const RESUME_BRANCH_TARGET: u64 = 4_000_000_000;

/// The kinds of message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ready,
    Stopped,
    Resume,
    Shutdown,
    Pause,
}

impl Kind {
    pub fn code(self) -> u16 {
        match self {
            Kind::Ready => 1,
            Kind::Stopped => 2,
            Kind::Resume => 3,
            Kind::Shutdown => 4,
            Kind::Pause => 5,
        }
    }

    pub fn from_code(code: u16) -> Option<Kind> {
        match code {
            1 => Some(Kind::Ready),
            2 => Some(Kind::Stopped),
            3 => Some(Kind::Resume),
            4 => Some(Kind::Shutdown),
            5 => Some(Kind::Pause),
            _ => None,
        }
    }
}

/// Why a message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The buffer is not exactly [`SIZE`] bytes.
    Length(usize),
    /// The magic number or the version is not the one this crate speaks.
    Protocol,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Length(n) => write!(f, "a control message is {SIZE} bytes, got {n}"),
            WireError::Protocol => {
                write!(f, "control message with an unknown magic number or version")
            }
        }
    }
}

impl std::error::Error for WireError {}

/// One message. The type is kept as its raw code: a message of a kind this crate does not know is read, then ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Message {
    pub kind: u16,
    pub stop_seq: u64,
    pub branch_count: u64,
    pub vtime_ns: u64,
    pub probe_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub vcpu_id: u32,
}

impl Message {
    pub fn of(kind: Kind) -> Message {
        Message {
            kind: kind.code(),
            ..Message::default()
        }
    }

    /// The answer to `event`: its `stop_seq` and `vcpu_id` echoed, the fence target on a RESUME, a zero probe.
    pub fn answer(kind: Kind, event: &Message) -> Message {
        Message {
            kind: kind.code(),
            stop_seq: event.stop_seq,
            branch_count: if kind == Kind::Resume {
                RESUME_BRANCH_TARGET
            } else {
                0
            },
            vcpu_id: event.vcpu_id,
            ..Message::default()
        }
    }

    pub fn kind(&self) -> Option<Kind> {
        Kind::from_code(self.kind)
    }

    pub fn encode(&self) -> [u8; SIZE] {
        let mut out = [0u8; SIZE];
        out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        out[4..6].copy_from_slice(&VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&self.kind.to_le_bytes());
        let longs = [
            self.stop_seq,
            self.branch_count,
            self.vtime_ns,
            self.probe_id,
            self.arg0,
            self.arg1,
            self.arg2,
        ];
        for (i, v) in longs.iter().enumerate() {
            out[8 + i * 8..16 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        out[64..68].copy_from_slice(&self.vcpu_id.to_le_bytes());
        out
    }

    pub fn decode(raw: &[u8]) -> Result<Message, WireError> {
        if raw.len() != SIZE {
            return Err(WireError::Length(raw.len()));
        }
        let u32_at = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap_or([0; 4]));
        let u16_at = |at: usize| u16::from_le_bytes(raw[at..at + 2].try_into().unwrap_or([0; 2]));
        let u64_at = |at: usize| u64::from_le_bytes(raw[at..at + 8].try_into().unwrap_or([0; 8]));
        if u32_at(0) != MAGIC || u16_at(4) != VERSION {
            return Err(WireError::Protocol);
        }
        Ok(Message {
            kind: u16_at(6),
            stop_seq: u64_at(8),
            branch_count: u64_at(16),
            vtime_ns: u64_at(24),
            probe_id: u64_at(32),
            arg0: u64_at(40),
            arg1: u64_at(48),
            arg2: u64_at(56),
            vcpu_id: u32_at(64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Message {
        Message {
            kind: Kind::Ready.code(),
            stop_seq: 7,
            branch_count: 395_000_000,
            vtime_ns: 1_234_567,
            probe_id: 3,
            arg0: 9,
            arg1: 10,
            arg2: 11,
            vcpu_id: 2,
        }
    }

    #[test]
    fn a_message_is_72_bytes_and_round_trips() {
        let raw = sample().encode();
        assert_eq!(raw.len(), 72);
        assert_eq!(Message::decode(&raw), Ok(sample()));
    }

    #[test]
    fn the_layout_is_magic_version_type_then_little_endian_fields() {
        let raw = sample().encode();
        assert_eq!(&raw[0..4], &[0x31, 0x56, 0x52, 0x44]);
        assert_eq!(&raw[4..6], &[1, 0]);
        assert_eq!(&raw[6..8], &[1, 0]);
        assert_eq!(&raw[8..16], &7u64.to_le_bytes());
        assert_eq!(&raw[64..68], &2u32.to_le_bytes());
        assert_eq!(&raw[68..72], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_wrong_length_magic_or_version_is_refused() {
        assert_eq!(Message::decode(&[0; 64]), Err(WireError::Length(64)));
        let mut raw = sample().encode();
        raw[0] ^= 1;
        assert_eq!(Message::decode(&raw), Err(WireError::Protocol));
        let mut raw = sample().encode();
        raw[4] = 2;
        assert_eq!(Message::decode(&raw), Err(WireError::Protocol));
    }

    #[test]
    fn a_resume_echoes_the_stop_and_the_vcpu_and_sets_the_fence() {
        let event = sample();
        let resume = Message::answer(Kind::Resume, &event);
        assert_eq!(resume.kind(), Some(Kind::Resume));
        assert_eq!(resume.stop_seq, 7);
        assert_eq!(resume.vcpu_id, 2);
        assert_eq!(resume.branch_count, RESUME_BRANCH_TARGET);
        assert_eq!(resume.probe_id, 0);
        let shutdown = Message::answer(Kind::Shutdown, &event);
        assert_eq!(shutdown.branch_count, 0);
        assert_eq!(shutdown.stop_seq, 7);
    }

    #[test]
    fn an_unknown_type_decodes_with_no_kind() {
        let raw = Message {
            kind: 99,
            ..sample()
        }
        .encode();
        let m = Message::decode(&raw).unwrap();
        assert_eq!(m.kind(), None);
        for k in [
            Kind::Ready,
            Kind::Stopped,
            Kind::Resume,
            Kind::Shutdown,
            Kind::Pause,
        ] {
            assert_eq!(Kind::from_code(k.code()), Some(k));
        }
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(WireError::Length(3).to_string().contains("72"));
        assert!(WireError::Protocol.to_string().contains("magic"));
        assert_eq!(Message::of(Kind::Pause).kind, 5);
    }
}
