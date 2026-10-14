use crate::rng::Rng;
use crate::{ActionId, Sim};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Config {
    pub seed: u64,
    pub max_steps: u64,
    /// Length of one walk when restarting from the root.
    pub walk_len: usize,
    /// Length of one walk when branching from a snapshot.
    pub branch_len: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            seed: 1,
            max_steps: 1_000_000,
            walk_len: 60,
            branch_len: 12,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Failure {
    /// Strictly replayable path from the root to the violation.
    pub path: Vec<ActionId>,
    /// Total simulation steps spent finding it (cost metric).
    pub steps_executed: u64,
}

/// Take one random step. Returns `None` when nothing is enabled.
fn random_step<S: Sim>(state: &S, rng: &mut Rng) -> Option<(S, ActionId)> {
    let enabled = state.enabled();
    if enabled.is_empty() {
        return None;
    }
    let action = enabled[rng.below(enabled.len())];
    Some((state.apply(action), S::action_id(&action)))
}

pub fn random_restarts<S: Sim>(root: &S, cfg: &Config) -> Option<Failure> {
    let mut rng = Rng::new(cfg.seed);
    let mut steps = 0u64;
    while steps < cfg.max_steps {
        let mut state = root.clone();
        let mut path = Vec::new();
        for _ in 0..cfg.walk_len {
            let Some((next, id)) = random_step(&state, &mut rng) else {
                break;
            };
            state = next;
            path.push(id);
            steps += 1;
            if state.violated() {
                return Some(Failure {
                    path,
                    steps_executed: steps,
                });
            }
        }
    }
    None
}

/// Snapshot-and-branch exploration: keep a corpus of states with novel signatures,
/// repeatedly restore the least-visited one and walk forward from it.
pub fn branching<S: Sim>(root: &S, cfg: &Config) -> Option<Failure> {
    let mut rng = Rng::new(cfg.seed);
    let mut steps = 0u64;
    let mut corpus: Vec<(S, Vec<ActionId>)> = vec![(root.clone(), Vec::new())];
    let mut visits: HashMap<u64, u32> = HashMap::from([(root.signature(), 1)]);
    while steps < cfg.max_steps {
        let (a, b) = (rng.below(corpus.len()), rng.below(corpus.len()));
        let pick = if visits[&corpus[a].0.signature()] <= visits[&corpus[b].0.signature()] {
            a
        } else {
            b
        };
        let (mut state, mut path) = corpus[pick].clone();
        for _ in 0..cfg.branch_len {
            let Some((next, id)) = random_step(&state, &mut rng) else {
                break;
            };
            state = next;
            path.push(id);
            steps += 1;
            if state.violated() {
                return Some(Failure {
                    path,
                    steps_executed: steps,
                });
            }
            let seen = visits.entry(state.signature()).or_insert(0);
            if *seen == 0 {
                corpus.push((state.clone(), path.clone()));
            }
            *seen += 1;
        }
    }
    None
}
