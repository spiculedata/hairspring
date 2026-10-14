//! Built-in demo targets so a fresh install has something to run.
use crate::erased::Registry;
use hairspring_core::demo::LockLease;
use hairspring_pubsub::{Mode, Order, PubSub};

pub fn default_registry() -> Registry {
    Registry::new()
        .with(
            "lock-lease",
            "Lease-based lock service; a paused client writes after its lease expired (split brain)",
            LockLease::new,
        )
        .with(
            "pubsub-buggy",
            "At-least-once consumer; check, apply and record are separate steps (duplicate effect)",
            || PubSub::new(Mode::Buggy, Order::Sorted),
        )
        .with(
            "pubsub-atomic",
            "Same consumer with atomic check+apply+record (the fix; should pass)",
            || PubSub::new(Mode::Atomic, Order::Sorted),
        )
}
