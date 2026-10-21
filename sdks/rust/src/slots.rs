//! The fixed table of per-thread slots. A thread claims one on its first SDK call; slots are never recycled and keys
//! cannot be reused, so the table suits fixed pools (see docs/rust-sdk.md).

use std::cell::Cell;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicU64, AtomicU8};
use std::sync::Mutex;

use crate::fmt::{is_ident_char, THREAD_KEY_MAX};
use crate::site::Witness;

/// Slots in the table; the last is shared by every thread past the 255th.
pub const MAX_THREADS: usize = 256;
pub const OVERFLOW_SLOT: usize = MAX_THREADS - 1;
const OVERFLOW_KEY: &[u8] = b"t#overflow";
/// A key plus its terminating NUL, with room to spare (the protocol allows 64 bytes).
pub const KEY_BYTES: usize = 72;

/// Per-thread state. Call sites touch only `evals`, `index` and `scratch`.
#[repr(C, align(64))]
pub struct Slot {
    pub(crate) evals: AtomicU64,
    pub(crate) index: AtomicU64,
    /// Sink for the losing side of a witness select.
    pub(crate) scratch: Witness,
    pub(crate) work: AtomicU64,
    /// Bytes read from the tape stream; stays 0 until the tape API exists.
    pub(crate) tape_consumed: AtomicU64,
    /// 1 when the key was set by `thread`, 0 for a claim-order key.
    pub(crate) is_stable: AtomicU64,
    key: [AtomicU8; KEY_BYTES],
}

/// Why `set_key` refused a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The label is empty.
    Empty,
    /// `label-index` is longer than 64 bytes.
    TooLong,
    /// Another thread has the key.
    Taken,
    /// The thread shares the overflow slot, which cannot be named.
    Overflow,
}

impl Slot {
    pub const fn new() -> Self {
        Self {
            evals: AtomicU64::new(0),
            index: AtomicU64::new(0),
            scratch: Witness::new(),
            work: AtomicU64::new(0),
            tape_consumed: AtomicU64::new(0),
            is_stable: AtomicU64::new(0),
            key: [const { AtomicU8::new(0) }; KEY_BYTES],
        }
    }

    /// The key, once published: a slot counts only after its first byte is stored last, with release.
    pub fn key(&self) -> Option<Vec<u8>> {
        if self.key[0].load(Acquire) == 0 {
            return None;
        }
        let bytes: Vec<u8> = self
            .key
            .iter()
            .map(|b| b.load(Relaxed))
            .take_while(|&b| b != 0)
            .collect();
        Some(bytes)
    }

    /// Is the published key exactly `key`? Compares in place, without building a copy.
    fn key_is(&self, key: &[u8]) -> bool {
        if self.key[0].load(Acquire) == 0 || key.len() >= KEY_BYTES {
            return false;
        }
        key.iter()
            .enumerate()
            .all(|(i, &c)| self.key[i].load(Relaxed) == c)
            && self.key[key.len()].load(Relaxed) == 0
    }

    /// Write the key so that its first byte becomes non-zero last.
    fn publish_key(&self, key: &[u8]) {
        for (i, &c) in key.iter().enumerate().skip(1) {
            self.key[i].store(c, Relaxed);
        }
        self.key[key.len()].store(0, Relaxed);
        self.key[0].store(key[0], Release);
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

/// The table, its claim counter and the lock that serialises key assignment (`thread` is not a call site).
pub struct SlotTable {
    slots: [Slot; MAX_THREADS],
    claims: AtomicU64,
    key_lock: Mutex<()>,
}

impl SlotTable {
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; MAX_THREADS],
            claims: AtomicU64::new(0),
            key_lock: Mutex::new(()),
        }
    }

    /// Slots in use: claims so far, capped at the table size.
    pub fn used(&self) -> usize {
        let n = self.claims.load(Relaxed);
        if n < OVERFLOW_SLOT as u64 {
            n as usize
        } else {
            MAX_THREADS
        }
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots[..self.used()]
    }

    pub fn slot(&self, index: usize) -> &Slot {
        &self.slots[index.min(OVERFLOW_SLOT)]
    }

    /// Claim the next slot; the flag says the thread shares the overflow slot.
    pub fn claim(&self) -> (&Slot, bool) {
        let n = self.claims.fetch_add(1, Relaxed);
        let index = (n as usize).min(OVERFLOW_SLOT);
        let slot = &self.slots[index];
        if n <= OVERFLOW_SLOT as u64 {
            slot.index.store(index as u64, Relaxed);
            slot.is_stable.store(0, Relaxed);
            slot.publish_key(default_key(n).as_bytes());
        }
        (slot, n >= OVERFLOW_SLOT as u64)
    }

    /// Give `slot` the key `label-index`. Bytes of the label outside `[A-Za-z0-9._:/-]` become `_`.
    pub fn set_key(&self, slot: &Slot, label: &str, index: u32) -> Result<(), KeyError> {
        if label.is_empty() {
            return Err(KeyError::Empty);
        }
        let mut key = KeyBuf::new();
        for c in label.bytes() {
            key.push(if is_ident_char(c) { c } else { b'_' });
        }
        key.push(b'-');
        key.push_decimal(u64::from(index));
        if key.len > THREAD_KEY_MAX {
            return Err(KeyError::TooLong);
        }
        if slot.index.load(Relaxed) as usize == OVERFLOW_SLOT {
            return Err(KeyError::Overflow);
        }
        let _guard = self.key_lock.lock().unwrap_or_else(|e| e.into_inner());
        let taken = self
            .slots()
            .iter()
            .filter(|other| !std::ptr::eq(*other, slot))
            .any(|other| other.key_is(key.as_bytes()));
        if taken {
            return Err(KeyError::Taken);
        }
        slot.publish_key(key.as_bytes());
        slot.is_stable.store(1, Relaxed);
        Ok(())
    }
}

impl Default for SlotTable {
    fn default() -> Self {
        Self::new()
    }
}

static TABLE: SlotTable = SlotTable::new();

thread_local! {
    // A const initialiser without a destructor: reading it is a plain thread-local load, with no lazy setup and no
    // registration, so a call site never allocates or makes a system call to find its slot.
    static CURRENT: Cell<*const Slot> = const { Cell::new(std::ptr::null()) };
}

/// The process-wide table.
pub fn table() -> &'static SlotTable {
    &TABLE
}

/// The calling thread's slot, claimed on its first call.
#[inline(always)]
pub fn current() -> &'static Slot {
    let cached = CURRENT.with(Cell::get);
    if cached.is_null() {
        return claim_current();
    }
    // SAFETY: a non-null value was stored by `claim_current` from a reference into the static table.
    unsafe { &*cached }
}

/// The first call on a thread: start the SDK if nothing has, take a slot, remember it. A pure-memory path apart from
/// the one-off startup; it depends on the thread's history, never on a condition.
#[cold]
#[inline(never)]
fn claim_current() -> &'static Slot {
    crate::runtime::ensure_started();
    let (slot, shared) = TABLE.claim();
    if shared {
        crate::runtime::note_dropped(1); // a thread that shares the overflow slot
    }
    CURRENT.with(|c| c.set(slot));
    slot
}

/// A key under construction, on the stack: assigning a key must not allocate. Bytes past the buffer are counted but
/// not stored, so `len` still says the key was too long and `set_key` refuses it.
struct KeyBuf {
    bytes: [u8; MAX_KEY_BUILD],
    len: usize,
}

/// A maximal key (64 bytes) plus room for the `-` and a 20-digit index of an over-long one.
const MAX_KEY_BUILD: usize = THREAD_KEY_MAX + 1 + 20;

impl KeyBuf {
    fn new() -> Self {
        Self {
            bytes: [0; MAX_KEY_BUILD],
            len: 0,
        }
    }

    /// Past the end the byte is counted but not stored, so `len` still says the key was too long.
    fn push(&mut self, c: u8) {
        if self.len < MAX_KEY_BUILD {
            self.bytes[self.len] = c;
        }
        self.len += 1;
    }

    fn push_decimal(&mut self, mut v: u64) {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for &d in &digits[i..] {
            self.push(d);
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len.min(MAX_KEY_BUILD)]
    }
}

/// `t#<claim order + 1>`, or `t#overflow` for the shared last slot.
fn default_key(claim_order: u64) -> KeyBuf {
    let mut key = KeyBuf::new();
    if claim_order >= OVERFLOW_SLOT as u64 {
        OVERFLOW_KEY.iter().for_each(|&c| key.push(c));
    } else {
        key.push(b't');
        key.push(b'#');
        key.push_decimal(claim_order + 1);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_claim_slots_in_order_with_claim_order_keys() {
        let t = SlotTable::new();
        let (a, shared_a) = t.claim();
        let (b, shared_b) = t.claim();
        assert!(!shared_a && !shared_b);
        assert_eq!(a.key().unwrap(), b"t#1");
        assert_eq!(b.key().unwrap(), b"t#2");
        assert_eq!(b.index.load(Relaxed), 1);
        assert_eq!(t.used(), 2);
        assert_eq!(a.is_stable.load(Relaxed), 0);
    }

    #[test]
    fn a_slot_with_no_key_is_not_reported() {
        let t = SlotTable::new();
        assert!(t.slot(5).key().is_none());
    }

    #[test]
    fn threads_past_the_table_share_the_last_slot_and_are_flagged() {
        let t = SlotTable::new();
        let mut shared = 0;
        for _ in 0..300 {
            if t.claim().1 {
                shared += 1;
            }
        }
        assert_eq!(t.used(), MAX_THREADS);
        assert_eq!(shared, 300 - OVERFLOW_SLOT);
        assert_eq!(t.slot(OVERFLOW_SLOT).key().unwrap(), b"t#overflow");
        assert_eq!(
            t.slot(OVERFLOW_SLOT).index.load(Relaxed),
            OVERFLOW_SLOT as u64
        );
        assert_eq!(t.slot(254).key().unwrap(), b"t#255");
    }

    #[test]
    fn a_thread_can_name_itself() {
        let t = SlotTable::new();
        let (s, _) = t.claim();
        assert_eq!(t.set_key(s, "writer", 3), Ok(()));
        assert_eq!(s.key().unwrap(), b"writer-3");
        assert_eq!(s.is_stable.load(Relaxed), 1);
    }

    #[test]
    fn label_bytes_outside_the_alphabet_become_underscores() {
        let t = SlotTable::new();
        let (s, _) = t.claim();
        assert_eq!(t.set_key(s, "a b#c", 0), Ok(()));
        assert_eq!(s.key().unwrap(), b"a_b_c-0");
    }

    #[test]
    fn a_key_cannot_be_taken_twice_but_can_be_retaken_by_its_owner() {
        let t = SlotTable::new();
        let (a, _) = t.claim();
        let (b, _) = t.claim();
        assert_eq!(t.set_key(a, "w", 1), Ok(()));
        assert_eq!(t.set_key(b, "w", 1), Err(KeyError::Taken));
        assert_eq!(
            b.key().unwrap(),
            b"t#2",
            "a refused key leaves the claim-order key"
        );
        assert_eq!(b.is_stable.load(Relaxed), 0);
        assert_eq!(t.set_key(a, "w", 1), Ok(()));
    }

    #[test]
    fn empty_and_overlong_labels_are_refused() {
        let t = SlotTable::new();
        let (s, _) = t.claim();
        assert_eq!(t.set_key(s, "", 0), Err(KeyError::Empty));
        assert_eq!(t.set_key(s, &"x".repeat(63), 0), Err(KeyError::TooLong));
        assert_eq!(t.set_key(s, &"x".repeat(62), 0), Ok(()));
    }

    #[test]
    fn the_shared_overflow_slot_cannot_be_named() {
        let t = SlotTable::new();
        for _ in 0..OVERFLOW_SLOT {
            t.claim();
        }
        let (s, shared) = t.claim();
        assert!(shared);
        assert_eq!(t.set_key(s, "late", 0), Err(KeyError::Overflow));
    }

    #[test]
    fn simultaneous_first_claims_get_distinct_slots() {
        let t = SlotTable::new();
        let indexes = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..32)
                .map(|_| scope.spawn(|| t.claim().0.index.load(Relaxed)))
                .collect();
            let mut got: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            got.sort_unstable();
            got
        });
        assert_eq!(indexes, (0..32).collect::<Vec<u64>>());
    }
}
