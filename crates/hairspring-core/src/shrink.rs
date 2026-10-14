use crate::replay::find_action;
use crate::{ActionId, Sim};

/// Run `path`, skipping actions that are not enabled. Returns whether it violated
/// and the actions that were actually applied (a strictly replayable path).
fn run_lenient<S: Sim>(root: &S, path: &[ActionId]) -> (bool, Vec<ActionId>) {
    let mut state = root.clone();
    let mut applied = Vec::with_capacity(path.len());
    for &id in path {
        if state.violated() {
            break;
        }
        if let Some(action) = find_action(&state, id) {
            state = state.apply(action);
            applied.push(id);
        }
    }
    (state.violated(), applied)
}

fn without_chunk(path: &[ActionId], start: usize, len: usize) -> Vec<ActionId> {
    path.iter()
        .enumerate()
        .filter(|(i, _)| *i < start || *i >= start + len)
        .map(|(_, id)| *id)
        .collect()
}

/// Delta-debugging style minimisation (ddmin): drop chunks of halving size until
/// no single step can be removed. Returns `path` unchanged if it does not fail.
pub fn shrink<S: Sim>(root: &S, path: &[ActionId]) -> Vec<ActionId> {
    let (fails, mut current) = run_lenient(root, path);
    if !fails {
        return path.to_vec();
    }
    loop {
        let mut changed = false;
        let mut chunk = (current.len() / 2).max(1);
        loop {
            let mut start = 0;
            while start < current.len() {
                let (still_fails, applied) =
                    run_lenient(root, &without_chunk(&current, start, chunk));
                if still_fails {
                    current = applied;
                    changed = true;
                } else {
                    start += chunk;
                }
            }
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
        if !changed {
            return current;
        }
    }
}
