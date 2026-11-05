//! A small, shared cap on replays: each one runs target code synchronously on the host, so a server answers a
//! replay over the cap at once (busy, try again later) instead of queueing it behind the others.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Replays allowed to run at once when nothing else is configured.
pub const DEFAULT_MAX_REPLAYS: usize = 2;
/// What a refused replay is told to wait, in seconds.
pub const REPLAY_RETRY_AFTER_SECS: u64 = 5;

/// A counting limit on replays. Clones share one count, so the REST API and the MCP endpoint draw on the same slots.
#[derive(Clone)]
pub struct ReplaySlots {
    inner: Arc<Count>,
}

struct Count {
    taken: AtomicUsize,
    max: usize,
}

/// One running replay. The slot is given back when this is dropped.
pub struct ReplayPermit {
    inner: Arc<Count>,
}

impl ReplaySlots {
    pub fn new(max: usize) -> ReplaySlots {
        ReplaySlots {
            inner: Arc::new(Count {
                taken: AtomicUsize::new(0),
                max,
            }),
        }
    }

    /// A permit if a slot is free, otherwise `None`. Never waits.
    pub fn try_acquire(&self) -> Option<ReplayPermit> {
        let taken = self.inner.taken.fetch_add(1, Ordering::SeqCst);
        if taken >= self.inner.max {
            self.inner.taken.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(ReplayPermit {
            inner: self.inner.clone(),
        })
    }
}

impl Drop for ReplayPermit {
    fn drop(&mut self) {
        self.inner.taken.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_are_counted_across_clones_and_returned_on_drop() {
        let slots = ReplaySlots::new(2);
        let other = slots.clone();
        let a = slots.try_acquire().unwrap();
        let _b = other.try_acquire().unwrap();
        assert!(slots.try_acquire().is_none());
        assert!(other.try_acquire().is_none());
        drop(a);
        assert!(other.try_acquire().is_some());
    }
}
