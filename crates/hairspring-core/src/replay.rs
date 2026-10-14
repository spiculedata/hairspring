use crate::{ActionId, Sim};

#[derive(Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The recorded action was not enabled at `step`: the build or the target changed.
    NotEnabled { step: usize, id: ActionId },
}

pub struct Replayed<S> {
    pub state: S,
    pub applied: usize,
}

pub fn find_action<S: Sim>(state: &S, id: ActionId) -> Option<S::Action> {
    state.enabled().into_iter().find(|a| S::action_id(a) == id)
}

/// Strict replay; stops at the first violation.
pub fn replay<S: Sim>(root: &S, path: &[ActionId]) -> Result<Replayed<S>, ReplayError> {
    let mut state = root.clone();
    for (step, &id) in path.iter().enumerate() {
        if state.violated() {
            return Ok(Replayed {
                state,
                applied: step,
            });
        }
        let action = find_action(&state, id).ok_or(ReplayError::NotEnabled { step, id })?;
        state = state.apply(action);
    }
    let applied = path.len();
    Ok(Replayed { state, applied })
}
