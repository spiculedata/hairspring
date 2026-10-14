//! The delivery engine: publishes a workload to the app with seeded faults and checks invariants.
use crate::faults::chance;
use crate::http::request;
use crate::launcher::AppLauncher;
use crate::scenario::Scenario;
use crate::sidecar::{start_sidecar, SidecarHandle};
use anthill_core::rng::Rng;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const SCHED_SALT: u64 = 0x9E37_79B9_7F4A_7C15;
const DEFAULT_STARTUP: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(50);
const MAX_CHECK_TEXT: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub delivered: u64,
    pub redelivered: u64,
    pub duplicates_injected: u64,
    pub crashes: u64,
    pub published_by_app: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Ok(Report),
    /// An invariant was violated: a finding about the app.
    Violation(String),
    /// The simulation could not run: not a finding.
    Error(String),
}

struct Subscription {
    pubsub: String,
    topic: String,
    route: String,
}

struct Msg {
    pubsub: String,
    topic: String,
    data: Value,
    attempts: u32,
    successes: u32,
    dropped: bool,
}

fn app_addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn wait_ready(port: u16, timeout: Duration) -> Result<Vec<Subscription>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(r) = request(
            app_addr(port),
            "GET",
            "/dapr/subscribe",
            &[],
            b"",
            Duration::from_secs(2),
        ) {
            if r.status == 200 {
                let v: Value = serde_json::from_slice(&r.body)
                    .map_err(|e| format!("/dapr/subscribe did not return JSON: {e}"))?;
                let subs = v
                    .as_array()
                    .ok_or("/dapr/subscribe must return a JSON array")?;
                return Ok(subs
                    .iter()
                    .filter_map(|s| {
                        Some(Subscription {
                            pubsub: s["pubsubname"].as_str()?.to_string(),
                            topic: s["topic"].as_str()?.to_string(),
                            route: s["route"]
                                .as_str()
                                .or_else(|| s["routes"]["default"].as_str())?
                                .to_string(),
                        })
                    })
                    .collect());
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the app did not become ready on port {port} within {}s",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(READY_POLL);
    }
}

enum Delivery {
    Success,
    Retry,
    Dropped,
}

fn deliver(port: u16, sub: &Subscription, index: usize, msg: &Msg, timeout: Duration) -> Delivery {
    let event = json!({ "id": format!("anthill-{index}"), "source": "anthill-dapr-sim", "type": "com.dapr.event.sent", "specversion": "1.0",
        "datacontenttype": "application/json", "topic": msg.topic, "pubsubname": msg.pubsub, "data": msg.data });
    let headers = [("Content-Type", "application/cloudevents+json")];
    match request(
        app_addr(port),
        "POST",
        &sub.route,
        &headers,
        event.to_string().as_bytes(),
        timeout,
    ) {
        Err(_) => Delivery::Retry,
        Ok(r) if (200..300).contains(&r.status) => match serde_json::from_slice::<Value>(&r.body)
            .ok()
            .and_then(|v| v["status"].as_str().map(str::to_uppercase))
        {
            Some(s) if s == "RETRY" => Delivery::Retry,
            Some(s) if s == "DROP" => Delivery::Dropped,
            _ => Delivery::Success,
        },
        Ok(r) if r.status == 404 => Delivery::Dropped,
        Ok(_) => Delivery::Retry,
    }
}

fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(MAX_CHECK_TEXT)
        .collect()
}

struct Run<'a> {
    seed: u64,
    scenario: &'a Scenario,
    sidecar: &'a SidecarHandle,
}

fn drive(run: &Run, app: &mut dyn AppLauncher) -> Verdict {
    let faults = &run.scenario.faults;
    let limits = &run.scenario.limits;
    let mut rng = Rng::new(run.seed ^ SCHED_SALT);
    let mut report = Report::default();
    let startup = run
        .scenario
        .app
        .startup_timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_STARTUP);
    let timeout = Duration::from_secs(limits.delivery_timeout_secs.max(1));

    let mut port = match app.start(run.sidecar.port()) {
        Ok(p) => p,
        Err(e) => return Verdict::Error(format!("could not start the app: {e}")),
    };
    let mut subs = match wait_ready(port, startup) {
        Ok(s) => s,
        Err(e) => return Verdict::Error(e),
    };
    let mut msgs: Vec<Msg> = run
        .scenario
        .messages()
        .into_iter()
        .map(|(pubsub, topic, data)| Msg {
            pubsub,
            topic,
            data,
            attempts: 0,
            successes: 0,
            dropped: false,
        })
        .collect();
    let mut duplicates: Vec<usize> = Vec::new();

    for _ in 0..limits.max_rounds {
        for (pubsub, topic, data) in run.sidecar.take_published() {
            report.published_by_app += 1;
            msgs.push(Msg {
                pubsub,
                topic,
                data,
                attempts: 0,
                successes: 0,
                dropped: false,
            });
        }
        let pending: Vec<usize> = (0..msgs.len())
            .filter(|&i| {
                msgs[i].successes == 0 && !msgs[i].dropped && msgs[i].attempts < limits.max_attempts
            })
            .collect();
        let (index, is_duplicate) =
            if !duplicates.is_empty() && (pending.is_empty() || chance(&mut rng, 0.5)) {
                (duplicates.remove(0), true)
            } else if pending.is_empty() {
                break;
            } else if chance(&mut rng, faults.reorder) {
                (pending[rng.below(pending.len())], false)
            } else {
                (pending[0], false)
            };
        let Some(sub) = subs
            .iter()
            .find(|s| s.pubsub == msgs[index].pubsub && s.topic == msgs[index].topic)
        else {
            return Verdict::Violation(format!(
                "the app has no subscription for topic `{}` on `{}`",
                msgs[index].topic, msgs[index].pubsub
            ));
        };
        msgs[index].attempts += 1;
        if chance(&mut rng, faults.drop) {
            continue; // lost in transit; it will be retried
        }
        match deliver(port, sub, index, &msgs[index], timeout) {
            Delivery::Success => {
                msgs[index].successes += 1;
                report.delivered += 1;
                if is_duplicate {
                    report.redelivered += 1;
                } else if chance(&mut rng, faults.duplicate) {
                    duplicates.push(index);
                    report.duplicates_injected += 1;
                }
            }
            Delivery::Dropped => msgs[index].dropped = true,
            Delivery::Retry => {}
        }
        if chance(&mut rng, faults.crash_app) {
            report.crashes += 1;
            app.stop();
            port = match app.start(run.sidecar.port()) {
                Ok(p) => p,
                Err(e) => return Verdict::Error(format!("could not restart the app: {e}")),
            };
            subs = match wait_ready(port, startup) {
                Ok(s) => s,
                Err(e) => return Verdict::Error(e),
            };
        }
    }

    for (i, m) in msgs.iter().enumerate() {
        if m.successes == 0 {
            let why = if m.dropped {
                "the app dropped it".to_string()
            } else {
                format!("{} delivery attempts failed", m.attempts)
            };
            return Verdict::Violation(format!(
                "message {i} on topic `{}` was never processed ({why})",
                m.topic
            ));
        }
    }
    match request(app_addr(port), "GET", "/anthill/check", &[], b"", timeout) {
        Ok(r) if r.status == 200 || r.status == 404 => Verdict::Ok(report),
        Ok(r) => Verdict::Violation(clip(&String::from_utf8_lossy(&r.body)).trim().to_string())
            .into_non_empty(r.status),
        Err(e) => Verdict::Error(format!("the app's /anthill/check endpoint failed: {e}")),
    }
}

trait NonEmpty {
    fn into_non_empty(self, status: u16) -> Verdict;
}

impl NonEmpty for Verdict {
    fn into_non_empty(self, status: u16) -> Verdict {
        match self {
            Verdict::Violation(m) if m.is_empty() => {
                Verdict::Violation(format!("the app's invariant check failed (HTTP {status})"))
            }
            other => other,
        }
    }
}

/// Runs one simulation. The fault plan and delivery order are a function of `seed`.
pub fn run_simulation(seed: u64, scenario: &Scenario, app: &mut dyn AppLauncher) -> Verdict {
    let sidecar = match start_sidecar("127.0.0.1:0", seed, scenario.faults.clone()) {
        Ok(s) => s,
        Err(e) => return Verdict::Error(format!("could not start the simulated sidecar: {e}")),
    };
    let verdict = drive(
        &Run {
            seed,
            scenario,
            sidecar: &sidecar,
        },
        app,
    );
    app.stop();
    sidecar.stop();
    verdict
}
