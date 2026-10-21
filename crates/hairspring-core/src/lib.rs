//! hairspring-core: explore many futures of a deterministic simulation, replay any
//! failure exactly, shrink it to a minimal trace, and verify determinism.
pub mod bundle;
pub mod demo;
pub mod doctor;
pub mod environment;
pub mod explore;
pub mod point;
pub mod replay;
pub mod rng;
pub mod shrink;
pub mod sim;

pub use sim::{stable_id, ActionId, Sim};
