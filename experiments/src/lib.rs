// Research crate from the early determinism experiments (see SPEC.md section 2). Kept for
// reproducibility of the published numbers; deliberately simple, not production code.
#![allow(
    clippy::should_implement_trait,
    clippy::new_without_default,
    clippy::unnecessary_map_or
)]
//! Toy deterministic world: a lease-based lock service with a classic
//! "stale lease holder writes after a pause" bug (no fencing token).
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const LEASE: u32 = 3;
pub const MAX_TIME: u32 = 8;

/// splitmix64: tiny, fast, fully deterministic given a seed.
#[derive(Clone)]
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    Idle,
    Waiting,
    Holding,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Client {
    pub phase: Phase,
    pub paused: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Tick,
    Acquire(usize),
    Deliver(usize),
    Drop(usize),
    Pause(usize),
    Resume(usize),
    Write(usize),
}

/// Whole world is plain data: `clone()` IS the snapshot, which is what
/// makes branching ("time travel") free here.
#[derive(Clone, Debug)]
pub struct World {
    pub time: u32,
    pub clients: [Client; 2],
    pub holder: Option<(usize, u32)>,
    pub inflight: Vec<usize>,
    pub violated: bool,
}

impl World {
    pub fn new() -> Self {
        World {
            time: 0,
            clients: [Client {
                phase: Phase::Idle,
                paused: false,
            }; 2],
            holder: None,
            inflight: vec![],
            violated: false,
        }
    }

    pub fn enabled(&self) -> Vec<Action> {
        let mut a = vec![];
        if self.time < MAX_TIME {
            a.push(Action::Tick);
        }
        for (i, c) in self.clients.iter().enumerate() {
            if c.phase == Phase::Idle {
                a.push(Action::Acquire(i));
            }
            if c.phase == Phase::Holding && !c.paused {
                a.push(Action::Pause(i));
                a.push(Action::Write(i));
            }
            if c.paused {
                a.push(Action::Resume(i));
            }
        }
        for k in 0..self.inflight.len() {
            a.push(Action::Deliver(k));
            a.push(Action::Drop(k));
        }
        a
    }

    pub fn apply(&mut self, act: Action) {
        match act {
            Action::Tick => self.time += 1,
            Action::Acquire(i) => {
                let free = self.holder.map_or(true, |(_, exp)| self.time >= exp);
                if free {
                    self.holder = Some((i, self.time + LEASE));
                    self.clients[i].phase = Phase::Waiting;
                    self.inflight.push(i);
                }
            }
            Action::Deliver(k) => {
                let i = self.inflight.remove(k);
                self.clients[i].phase = Phase::Holding;
            }
            Action::Drop(k) => {
                let i = self.inflight.remove(k);
                self.clients[i].phase = Phase::Idle;
            }
            Action::Pause(i) => self.clients[i].paused = true,
            Action::Resume(i) => self.clients[i].paused = false,
            Action::Write(i) => {
                // BUG: client trusts its stale belief; server-side check shows the truth.
                let valid = matches!(self.holder, Some((h, exp)) if h == i && self.time < exp);
                let other_holds = self.clients[1 - i].phase == Phase::Holding;
                // Split-brain: my lease is dead AND someone else currently believes they hold the lock.
                if !valid && other_holds {
                    self.violated = true;
                }
                self.clients[i].phase = Phase::Done;
            }
        }
    }

    /// Abstract state: what an exploration heuristic uses as "coverage".
    pub fn signature(&self) -> u64 {
        let rem = self.holder.map(|(h, e)| (h, e.saturating_sub(self.time)));
        let mut h = DefaultHasher::new();
        (self.clients, rem, self.inflight.len()).hash(&mut h);
        h.finish()
    }
}

pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}
