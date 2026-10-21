//! A small instrumented program: a bounded queue shared by a producer and two consumers.
//!
//! Run it and read the report on stderr:
//!
//! ```text
//! cargo run -p hairspring-sdk --example quickstart 2>&1 >/dev/null | head
//! ```
//!
//! Set `HAIRSPRING_EVENTS_FILE=events.ndjson` to send the report to a file instead, or `HAIRSPRING_EVENTS=off` to
//! silence it. Under Hairspring the report is folded into verdicts: `always!` fails a trial when it is false,
//! `sometimes!` and `reachable!` are coverage the whole run must meet.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

use hairspring_sdk::{always, reachable, sometimes, thread, work};

const CAPACITY: usize = 4;
const ITEMS: u32 = 100;
const CONSUMERS: u32 = 2;

struct Queue {
    items: Mutex<VecDeque<u32>>,
    changed: Condvar,
}

fn produce(queue: &Queue) {
    for item in 0..ITEMS {
        let mut items = queue.items.lock().expect("queue lock");
        while items.len() >= CAPACITY {
            // The producer had to wait for a consumer: worth knowing that a run got here at least once.
            sometimes!(
                true,
                "queue.producer_waited",
                "the producer found the queue full"
            );
            items = queue.changed.wait(items).expect("queue lock");
        }
        items.push_back(item);
        always!(
            items.len() <= CAPACITY,
            "queue.bounded",
            "the queue never exceeds its capacity",
            items.len()
        );
        queue.changed.notify_all();
    }
    // A sentinel per consumer.
    for _ in 0..CONSUMERS {
        queue.items.lock().expect("queue lock").push_back(u32::MAX);
        queue.changed.notify_all();
    }
}

fn consume(queue: &Queue, id: u32) {
    let _ = thread("consumer", id);
    loop {
        let mut items = queue.items.lock().expect("queue lock");
        let item = loop {
            if let Some(item) = items.pop_front() {
                break item;
            }
            items = queue.changed.wait(items).expect("queue lock");
        };
        queue.changed.notify_all();
        drop(items);
        if item == u32::MAX {
            reachable!("queue.consumer_done", "a consumer saw the end marker", id);
            return;
        }
        work(1);
    }
}

fn main() {
    let queue = Queue {
        items: Mutex::new(VecDeque::new()),
        changed: Condvar::new(),
    };
    std::thread::scope(|scope| {
        for id in 0..CONSUMERS {
            let queue = &queue;
            scope.spawn(move || consume(queue, id));
        }
        produce(&queue);
    });
    println!("done");
}
