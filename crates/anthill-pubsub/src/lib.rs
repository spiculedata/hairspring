//! U4 target: competing consumers on an at-least-once broker with a visibility
//! timeout, a dedupe table and crashes. Invariant: each message's effect happens once.
use anthill_core::{stable_id, ActionId, Sim};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// check, apply and record are separate steps (the classic bug).
    Buggy,
    /// check+apply+record happen atomically (unique-constraint style fix).
    Atomic,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Deterministic: deliverable messages enumerated in ascending id.
    Sorted,
    /// Naive: enumerated by iterating a freshly built HashMap (a realistic leak).
    HashMap,
}

pub const MSGS: usize = 2;
pub const CONSUMERS: usize = 2;
pub const VISIBILITY_TIMEOUT: u32 = 2;
pub const MAX_TIME: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Tick,
    Deliver(u8, u8),
    Step(u8),
    Crash(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Received,
    Checked,
    Applied,
    Recorded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Consumer {
    Idle,
    Busy { msg: u8, stage: Stage },
}

#[derive(Clone, Copy, Debug)]
pub struct BrokerMsg {
    pub acked: bool,
    pub visible_at: u32,
}

#[derive(Clone, Debug)]
pub struct PubSub {
    mode: Mode,
    order: Order,
    pub time: u32,
    pub broker: [BrokerMsg; MSGS],
    pub consumers: [Consumer; CONSUMERS],
    pub processed: [bool; MSGS],
    pub effects: [u8; MSGS],
    pub violated: bool,
}

impl PubSub {
    pub fn new(mode: Mode, order: Order) -> Self {
        PubSub {
            mode,
            order,
            time: 0,
            broker: [BrokerMsg {
                acked: false,
                visible_at: 0,
            }; MSGS],
            consumers: [Consumer::Idle; CONSUMERS],
            processed: [false; MSGS],
            effects: [0; MSGS],
            violated: false,
        }
    }

    fn deliverable(&self) -> Vec<u8> {
        let ids = (0..MSGS as u8).filter(|&m| {
            let b = self.broker[m as usize];
            !b.acked && b.visible_at <= self.time
        });
        match self.order {
            Order::Sorted => ids.collect(),
            // The leak: ordinary-looking code, iteration order differs on every call.
            Order::HashMap => {
                let set: std::collections::HashMap<u8, ()> = ids.map(|m| (m, ())).collect();
                set.keys().copied().collect()
            }
        }
    }

    fn with_consumer(&self, c: u8, consumer: Consumer) -> Self {
        let mut next = self.clone();
        next.consumers[c as usize] = consumer;
        next
    }

    fn ack(&self, c: u8, msg: u8) -> Self {
        let mut next = self.with_consumer(c, Consumer::Idle);
        next.broker[msg as usize].acked = true;
        next
    }

    fn apply_effect(&self, msg: u8) -> Self {
        let mut next = self.clone();
        next.effects[msg as usize] += 1;
        next.violated = next.violated || next.effects[msg as usize] > 1;
        next
    }

    fn step(&self, c: u8, msg: u8, stage: Stage) -> Self {
        let busy = |stage| Consumer::Busy { msg, stage };
        let m = msg as usize;
        match (self.mode, stage) {
            // Atomic: check+apply+record in one step (unique-constraint style).
            (Mode::Atomic, Stage::Received) => {
                let mut next = if self.processed[m] {
                    self.clone()
                } else {
                    self.apply_effect(msg)
                };
                next.processed[m] = true;
                next.with_consumer(c, busy(Stage::Recorded))
            }
            (Mode::Buggy, Stage::Received) if self.processed[m] => self.ack(c, msg),
            (Mode::Buggy, Stage::Received) => self.with_consumer(c, busy(Stage::Checked)),
            (Mode::Buggy, Stage::Checked) => self
                .apply_effect(msg)
                .with_consumer(c, busy(Stage::Applied)),
            (Mode::Buggy, Stage::Applied) => {
                let mut next = self.with_consumer(c, busy(Stage::Recorded));
                next.processed[m] = true;
                next
            }
            (_, Stage::Recorded) => self.ack(c, msg),
            (Mode::Atomic, _) => self.clone(),
        }
    }
}

impl Sim for PubSub {
    type Action = Action;

    fn enabled(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.time < MAX_TIME {
            actions.push(Action::Tick);
        }
        let deliverable = self.deliverable();
        for (c, consumer) in self.consumers.iter().enumerate() {
            let c = c as u8;
            match consumer {
                Consumer::Idle => {
                    actions.extend(deliverable.iter().map(|&m| Action::Deliver(c, m)))
                }
                Consumer::Busy { .. } => actions.extend([Action::Step(c), Action::Crash(c)]),
            }
        }
        actions
    }

    fn action_id(a: &Action) -> ActionId {
        match *a {
            Action::Tick => stable_id(&[0, 0, 0]),
            Action::Deliver(c, m) => stable_id(&[1, c as u64, m as u64]),
            Action::Step(c) => stable_id(&[2, c as u64, 0]),
            Action::Crash(c) => stable_id(&[3, c as u64, 0]),
        }
    }

    fn apply(&self, a: Action) -> Self {
        match a {
            Action::Tick => {
                let mut next = self.clone();
                next.time += 1;
                next
            }
            Action::Deliver(c, m) => {
                let mut next = self.with_consumer(
                    c,
                    Consumer::Busy {
                        msg: m,
                        stage: Stage::Received,
                    },
                );
                next.broker[m as usize].visible_at = self.time + VISIBILITY_TIMEOUT;
                next
            }
            Action::Step(c) => match self.consumers[c as usize] {
                Consumer::Busy { msg, stage } => self.step(c, msg, stage),
                Consumer::Idle => self.clone(),
            },
            Action::Crash(c) => self.with_consumer(c, Consumer::Idle),
        }
    }

    fn signature(&self) -> u64 {
        let mut parts: Vec<u64> = Vec::new();
        for c in &self.consumers {
            parts.extend(match c {
                Consumer::Idle => [9, 9],
                Consumer::Busy { msg, stage } => [*msg as u64, *stage as u64],
            });
        }
        for (m, b) in self.broker.iter().enumerate() {
            parts.extend([
                b.acked as u64,
                b.visible_at.saturating_sub(self.time) as u64,
                self.processed[m] as u64,
                self.effects[m] as u64,
            ]);
        }
        stable_id(&parts)
    }

    fn violated(&self) -> bool {
        self.violated
    }
}
