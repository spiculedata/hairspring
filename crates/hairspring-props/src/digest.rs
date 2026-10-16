//! The trial digest: everything the run fold needs from one trial, in a compact versioned binary form.
//!
//! A digest is what a worker sends and a server stores for every judged trial. It refers to ids by their index in the
//! run catalog ([`crate::RunCatalog`]), so a trial with 50 ids costs a few hundred bytes. The decoder is the trust
//! boundary: it takes bytes from an untrusted sender and refuses anything that is not exactly what [`TrialDigest::encode`]
//! writes, within the protocol's own limits.
//!
//! Layout (all integers are LEB128 varints, signed values zigzag):
//! `version(1) flags ids_dropped entry_count entry*`, where an entry is
//! `idx header [hits trues falses] [witness] [decl]` and the header byte holds the kind in bits 0-2 and the flags
//! catalogued (3), conflicted (4), witness (5) and declaration (6).

use crate::event::{AssertKind, Loc};
use crate::limits::{
    MAX_IDS_PER_RUN, MAX_LOC_FILE_BYTES, MAX_LOC_FUNC_BYTES, MAX_MSG_BYTES, MAX_PROC_BYTES,
    MAX_THREAD_KEY_BYTES, VALS_LEN,
};
use crate::trial::{failure_basis, Witness};
use crate::valid::{is_ident, is_plain_text, is_thread_key};
use std::collections::HashSet;
use std::fmt;

/// The only digest version this build reads and writes.
pub const DIGEST_VERSION: u8 = 1;
/// Largest digest accepted, in bytes. A trial within the event-line budget stays far below it.
pub const MAX_DIGEST_BYTES: usize = 1024 * 1024;
/// Most failure witnesses one digest carries. A trial with more failing ids keeps counts for the rest.
pub const MAX_DIGEST_WITNESSES: usize = 64;
/// Most kind-mismatch declarations one digest carries (the run keeps no more details than this).
pub const MAX_DIGEST_DECLS: usize = crate::run::MAX_KIND_MISMATCH_DETAILS;

const FLAG_INCONCLUSIVE: u8 = 1;
const FLAG_DEGRADED: u8 = 1 << 1;
const FLAG_PERTURBING: u8 = 1 << 2;
const FLAG_REPORT_MISSING: u8 = 1 << 3;
const FLAG_HAS_CONFLICTS: u8 = 1 << 4;
const FLAG_EXCLUDED: u8 = 1 << 5;
const FLAG_RESERVED: u8 = !(FLAG_INCONCLUSIVE
    | FLAG_DEGRADED
    | FLAG_PERTURBING
    | FLAG_REPORT_MISSING
    | FLAG_HAS_CONFLICTS
    | FLAG_EXCLUDED);

const KIND_MASK: u8 = 0b111;
const ENTRY_CATALOGUED: u8 = 1 << 3;
const ENTRY_CONFLICTED: u8 = 1 << 4;
const ENTRY_WITNESS: u8 = 1 << 5;
const ENTRY_DECL: u8 = 1 << 6;
const ENTRY_RESERVED: u8 = 1 << 7;

/// How the trial may be used, as the run fold sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigestFlags {
    /// The trial timed out: its report decides nothing.
    pub inconclusive: bool,
    /// Event data was lost or contradictory.
    pub degraded: bool,
    /// The trial contained per-evaluation `eval` lines.
    pub perturbing: bool,
    /// A process never sent `end`.
    pub report_missing: bool,
    /// The trial had at least one catalog conflict.
    pub has_conflicts: bool,
    /// The host could not collect this trial's events, so its data cannot be trusted: the trial is stored for the
    /// record and left out of the run's statistics entirely.
    pub excluded: bool,
}

impl DigestFlags {
    fn bits(self) -> u8 {
        [
            (self.inconclusive, FLAG_INCONCLUSIVE),
            (self.degraded, FLAG_DEGRADED),
            (self.perturbing, FLAG_PERTURBING),
            (self.report_missing, FLAG_REPORT_MISSING),
            (self.has_conflicts, FLAG_HAS_CONFLICTS),
            (self.excluded, FLAG_EXCLUDED),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit)
    }

    fn from_bits(b: u8) -> DigestFlags {
        DigestFlags {
            inconclusive: b & FLAG_INCONCLUSIVE != 0,
            degraded: b & FLAG_DEGRADED != 0,
            perturbing: b & FLAG_PERTURBING != 0,
            report_missing: b & FLAG_REPORT_MISSING != 0,
            has_conflicts: b & FLAG_HAS_CONFLICTS != 0,
            excluded: b & FLAG_EXCLUDED != 0,
        }
    }
}

/// A trial's own message and location for an id whose kind disagreed with the run's, kept for diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    pub msg: String,
    pub loc: Option<Loc>,
}

/// One id in one trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestEntry {
    /// Index into the run catalog.
    pub idx: u32,
    /// The kind this trial declared for the id.
    pub kind: AssertKind,
    /// The trial reported counts for the id. False for an entry that only marks a catalog conflict.
    pub catalogued: bool,
    /// The trial declared the id with two different kinds.
    pub conflicted: bool,
    pub hits: u64,
    pub trues: u64,
    pub falses: u64,
    /// Where the id failed first, when it failed in this trial (see [`MAX_DIGEST_WITNESSES`]).
    pub witness: Option<Witness>,
    pub decl: Option<Decl>,
}

/// One judged trial, reduced to what the run fold needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrialDigest {
    pub flags: DigestFlags,
    /// Ids the sender left out because the run already tracked its maximum.
    pub ids_dropped: u64,
    /// In the trial's own property order, then conflict-only entries. Indexes are unique.
    pub entries: Vec<DigestEntry>,
}

/// Why a digest was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestError(pub &'static str);

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trial digest: {}", self.0)
    }
}

impl std::error::Error for DigestError {}

type Res<T> = Result<T, DigestError>;

fn bad<T>(why: &'static str) -> Res<T> {
    Err(DigestError(why))
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn put_witness(out: &mut Vec<u8>, w: &Witness) {
    put_str(out, &w.proc);
    put_str(out, &w.thread);
    put_varint(out, w.ord);
    for v in w.vals {
        put_varint(out, zigzag(v));
    }
}

fn put_decl(out: &mut Vec<u8>, d: &Decl) {
    put_str(out, &d.msg);
    match &d.loc {
        None => out.push(0),
        Some(loc) => {
            out.push(1);
            put_str(out, &loc.file);
            put_varint(out, loc.line);
            match &loc.func {
                None => out.push(0),
                Some(f) => {
                    out.push(1);
                    put_str(out, f);
                }
            }
        }
    }
}

impl DigestEntry {
    fn header(&self) -> u8 {
        let kind = AssertKind::ALL
            .iter()
            .position(|k| *k == self.kind)
            .unwrap_or(0) as u8;
        [
            (self.catalogued, ENTRY_CATALOGUED),
            (self.conflicted, ENTRY_CONFLICTED),
            (self.witness.is_some(), ENTRY_WITNESS),
            (self.decl.is_some(), ENTRY_DECL),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(kind, |acc, (_, bit)| acc | bit)
    }
}

impl TrialDigest {
    /// The canonical bytes of this digest.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![DIGEST_VERSION, self.flags.bits()];
        put_varint(&mut out, self.ids_dropped);
        put_varint(&mut out, self.entries.len() as u64);
        for e in &self.entries {
            put_varint(&mut out, u64::from(e.idx));
            out.push(e.header());
            if e.catalogued {
                put_varint(&mut out, e.hits);
                put_varint(&mut out, e.trues);
                put_varint(&mut out, e.falses);
            }
            if let Some(w) = &e.witness {
                put_witness(&mut out, w);
            }
            if let Some(d) = &e.decl {
                put_decl(&mut out, d);
            }
        }
        out
    }

    /// Reads a digest from untrusted bytes, or says why not. Nothing is accepted that [`TrialDigest::encode`] would
    /// not write for the same value.
    pub fn decode(bytes: &[u8]) -> Res<TrialDigest> {
        if bytes.len() > MAX_DIGEST_BYTES {
            return bad("digest is larger than the size cap");
        }
        let mut r = Reader { buf: bytes, pos: 0 };
        if r.byte()? != DIGEST_VERSION {
            return bad("unsupported digest version");
        }
        let flag_bits = r.byte()?;
        if flag_bits & FLAG_RESERVED != 0 {
            return bad("reserved flag bits are set");
        }
        let ids_dropped = r.varint()?;
        let count = r.varint()?;
        if count > MAX_IDS_PER_RUN as u64 {
            return bad("too many entries");
        }
        let mut seen = HashSet::new();
        let mut entries = Vec::with_capacity((count as usize).min(bytes.len()));
        let (mut witnesses, mut decls) = (0usize, 0usize);
        for _ in 0..count {
            let e = r.entry()?;
            if !seen.insert(e.idx) {
                return bad("an index appears twice");
            }
            witnesses += usize::from(e.witness.is_some());
            decls += usize::from(e.decl.is_some());
            entries.push(e);
        }
        if witnesses > MAX_DIGEST_WITNESSES {
            return bad("too many witnesses");
        }
        if decls > MAX_DIGEST_DECLS {
            return bad("too many declarations");
        }
        if r.pos != bytes.len() {
            return bad("bytes follow the digest");
        }
        Ok(TrialDigest {
            flags: DigestFlags::from_bits(flag_bits),
            ids_dropped,
            entries,
        })
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Res<u8> {
        let b = *self
            .buf
            .get(self.pos)
            .ok_or(DigestError("digest is truncated"))?;
        self.pos += 1;
        Ok(b)
    }

    fn flag(&mut self) -> Res<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => bad("a presence byte is neither 0 nor 1"),
        }
    }

    fn varint(&mut self) -> Res<u64> {
        let mut value = 0u64;
        for i in 0..10u32 {
            let b = self.byte()?;
            if i == 9 && b > 1 {
                return bad("a number does not fit in 64 bits");
            }
            value |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                if b == 0 && i > 0 {
                    return bad("a number is not in its shortest form");
                }
                return Ok(value);
            }
        }
        bad("a number is too long")
    }

    fn text(&mut self, max: usize) -> Res<&str> {
        let len = self.varint()?;
        if len > max as u64 {
            return bad("a string is too long");
        }
        let end = self.pos + len as usize;
        let raw = self
            .buf
            .get(self.pos..end)
            .ok_or(DigestError("digest is truncated"))?;
        self.pos = end;
        std::str::from_utf8(raw).or(bad("a string is not UTF-8"))
    }

    fn witness(&mut self) -> Res<Witness> {
        let proc = self.text(MAX_PROC_BYTES)?.to_owned();
        let thread = self.text(MAX_THREAD_KEY_BYTES)?.to_owned();
        if !is_ident(&proc, MAX_PROC_BYTES) || !is_thread_key(&thread, MAX_THREAD_KEY_BYTES) {
            return bad("a witness names an invalid process or thread");
        }
        let ord = self.varint()?;
        let mut vals = [0i64; VALS_LEN];
        for v in &mut vals {
            *v = unzigzag(self.varint()?);
        }
        Ok(Witness {
            proc,
            thread,
            ord,
            vals,
        })
    }

    fn plain(&mut self, max: usize) -> Res<String> {
        let s = self.text(max)?;
        if !is_plain_text(s, max) {
            return bad("text has control or format characters");
        }
        Ok(s.to_owned())
    }

    fn decl(&mut self) -> Res<Decl> {
        let msg = self.plain(MAX_MSG_BYTES)?;
        let loc = if self.flag()? {
            let file = self.plain(MAX_LOC_FILE_BYTES)?;
            let line = self.varint()?;
            let func = if self.flag()? {
                Some(self.plain(MAX_LOC_FUNC_BYTES)?)
            } else {
                None
            };
            Some(Loc { file, line, func })
        } else {
            None
        };
        Ok(Decl { msg, loc })
    }

    fn entry(&mut self) -> Res<DigestEntry> {
        let idx = u32::try_from(self.varint()?).or(bad("an index is too large"))?;
        let header = self.byte()?;
        if header & ENTRY_RESERVED != 0 {
            return bad("a reserved entry bit is set");
        }
        let kind = *AssertKind::ALL
            .get(usize::from(header & KIND_MASK))
            .ok_or(DigestError("unknown assertion kind"))?;
        let (catalogued, conflicted) = (
            header & ENTRY_CATALOGUED != 0,
            header & ENTRY_CONFLICTED != 0,
        );
        if !catalogued && !conflicted {
            return bad("an entry says nothing");
        }
        let (mut hits, mut trues, mut falses) = (0, 0, 0);
        if catalogued {
            hits = self.varint()?;
            trues = self.varint()?;
            falses = self.varint()?;
            if trues.checked_add(falses) != Some(hits) {
                return bad("hits are not trues plus falses");
            }
        }
        let witness = if header & ENTRY_WITNESS != 0 {
            if !catalogued || failure_basis(kind, hits, falses).is_none() {
                return bad("a witness on an entry that did not fail");
            }
            Some(self.witness()?)
        } else {
            None
        };
        let decl = if header & ENTRY_DECL != 0 {
            if !catalogued {
                return bad("a declaration on an entry with no counts");
            }
            Some(self.decl()?)
        } else {
            None
        };
        Ok(DigestEntry {
            idx,
            kind,
            catalogued,
            conflicted,
            hits,
            trues,
            falses,
            witness,
            decl,
        })
    }
}
