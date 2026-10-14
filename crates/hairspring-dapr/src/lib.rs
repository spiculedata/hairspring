//! A simulated Dapr sidecar for deterministic fault testing of Dapr-based services.
//!
//! The simulator is a supervisor: it starts a seeded fake sidecar (state store with ETags,
//! pub/sub with at-least-once delivery), launches your app, drives a workload while injecting
//! faults (duplicates, reordering, drops, app crashes, state conflicts and errors), and checks
//! invariants. Run it as the command of a `process` target so [Hermit] explores its schedules:
//! the fault plan is seeded from `HAIRSPRING_SEED`, which the Hermit backend sets to the trial seed.
//!
//! Implemented subset of the Dapr HTTP API: state save/get/delete with ETags and first-write
//! concurrency, publish, and subscription delivery as CloudEvents (`/dapr/subscribe`). Bindings,
//! secrets, actors, workflows and gRPC are not simulated.
//!
//! [Hermit]: https://github.com/rrnewton/hermit
pub mod faults;
pub mod http;
pub mod launcher;
pub mod scenario;
pub mod sidecar;
pub mod sim;

pub use faults::FaultPlan;
pub use launcher::{AppLauncher, ProcessApp};
pub use scenario::Scenario;
pub use sidecar::{start_sidecar, start_sidecar_with, SidecarHandle, SidecarLimits};
pub use sim::{run_simulation, Report, Verdict};
