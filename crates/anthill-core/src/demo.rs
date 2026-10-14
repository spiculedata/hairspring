//! Demo target: a lease-based lock service with a stale-holder split-brain bug.
use crate::{stable_id, ActionId, Sim};

pub const LEASE: u32 = 3;
pub const MAX_TIME: u32 = 8;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Tick,
    Acquire(u8),
    Deliver(u8),
    Drop(u8),
    Pause(u8),
    Resume(u8),
    Write(u8),
}

#[derive(Clone, Debug)]
pub struct LockLease {
    pub time: u32,
    pub clients: [Client; 2],
    pub holder: Option<(u8, u32)>,
    pub violated: bool,
}

impl LockLease {
    pub fn new() -> Self {
        let idle = Client {
            phase: Phase::Idle,
            paused: false,
        };
        LockLease {
            time: 0,
            clients: [idle; 2],
            holder: None,
            violated: false,
        }
    }
}
impl Default for LockLease {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim for LockLease {
    type Action = Action;

    fn enabled(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.time < MAX_TIME {
            actions.push(Action::Tick);
        }
        for (i, c) in self.clients.iter().enumerate() {
            let i = i as u8;
            match (c.phase, c.paused) {
                (Phase::Idle, _) => actions.push(Action::Acquire(i)),
                (Phase::Waiting, _) => actions.extend([Action::Deliver(i), Action::Drop(i)]),
                (Phase::Holding, false) => actions.extend([Action::Pause(i), Action::Write(i)]),
                (Phase::Holding, true) => actions.push(Action::Resume(i)),
                (Phase::Done, _) => {}
            }
        }
        actions
    }

    fn action_id(a: &Action) -> ActionId {
        match *a {
            Action::Tick => stable_id(&[0, 0]),
            Action::Acquire(i) => stable_id(&[1, i as u64]),
            Action::Deliver(i) => stable_id(&[2, i as u64]),
            Action::Drop(i) => stable_id(&[3, i as u64]),
            Action::Pause(i) => stable_id(&[4, i as u64]),
            Action::Resume(i) => stable_id(&[5, i as u64]),
            Action::Write(i) => stable_id(&[6, i as u64]),
        }
    }

    fn apply(&self, a: Action) -> Self {
        let mut next = self.clone();
        match a {
            Action::Tick => next.time += 1,
            Action::Acquire(i) => {
                let free = self.holder.is_none_or(|(_, exp)| self.time >= exp);
                if free {
                    next.holder = Some((i, self.time + LEASE));
                    next.clients[i as usize].phase = Phase::Waiting;
                }
            }
            Action::Deliver(i) => next.clients[i as usize].phase = Phase::Holding,
            Action::Drop(i) => next.clients[i as usize].phase = Phase::Idle,
            Action::Pause(i) => next.clients[i as usize].paused = true,
            Action::Resume(i) => next.clients[i as usize].paused = false,
            Action::Write(i) => {
                // BUG: the client trusts its stale belief; the server's view is the truth.
                let valid = matches!(self.holder, Some((h, exp)) if h == i && self.time < exp);
                let other_holds = self.clients[1 - i as usize].phase == Phase::Holding;
                if !valid && other_holds {
                    next.violated = true;
                }
                next.clients[i as usize].phase = Phase::Done;
            }
        }
        next
    }

    fn signature(&self) -> u64 {
        let (holder, remaining) = self.holder.map_or((9, 0), |(h, e)| {
            (h as u64, e.saturating_sub(self.time) as u64)
        });
        let mut parts: Vec<u64> = vec![holder, remaining];
        parts.extend(
            self.clients
                .iter()
                .flat_map(|c| [c.phase as u64, c.paused as u64]),
        );
        stable_id(&parts)
    }

    fn violated(&self) -> bool {
        self.violated
    }
}
