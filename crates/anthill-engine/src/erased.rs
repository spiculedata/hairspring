//! Object-safe wrapper so any `Sim` can be registered by name and driven by
//! the generic algorithms in `anthill-core`.
use anthill_core::{ActionId, Sim};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

trait DynSim: Send + Sync {
    fn box_clone(&self) -> Box<dyn DynSim>;
    fn enabled_ids(&self) -> Vec<ActionId>;
    fn enabled_labeled(&self) -> Vec<(ActionId, String)>;
    fn apply_id(&self, id: ActionId) -> Option<Box<dyn DynSim>>;
    fn signature(&self) -> u64;
    fn violated(&self) -> bool;
    fn state_debug(&self) -> String;
}

struct Wrapper<S>(S);

impl<S: Sim + Debug + Send + Sync + 'static> DynSim for Wrapper<S> {
    fn box_clone(&self) -> Box<dyn DynSim> {
        Box::new(Wrapper(self.0.clone()))
    }
    fn enabled_ids(&self) -> Vec<ActionId> {
        self.0.enabled().iter().map(S::action_id).collect()
    }
    fn enabled_labeled(&self) -> Vec<(ActionId, String)> {
        self.0
            .enabled()
            .iter()
            .map(|a| (S::action_id(a), format!("{a:?}")))
            .collect()
    }
    fn apply_id(&self, id: ActionId) -> Option<Box<dyn DynSim>> {
        let action = self
            .0
            .enabled()
            .into_iter()
            .find(|a| S::action_id(a) == id)?;
        Some(Box::new(Wrapper(self.0.apply(action))))
    }
    fn signature(&self) -> u64 {
        self.0.signature()
    }
    fn violated(&self) -> bool {
        self.0.violated()
    }
    fn state_debug(&self) -> String {
        format!("{:?}", self.0)
    }
}

/// A type-erased simulation state. Cloning is the snapshot operation.
pub struct Erased(Box<dyn DynSim>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErasedAction(pub ActionId);

impl Erased {
    pub fn new<S: Sim + Debug + Send + Sync + 'static>(sim: S) -> Self {
        Erased(Box::new(Wrapper(sim)))
    }
    /// Human readable (action id, label) pairs for the currently enabled actions.
    pub fn enabled_labeled(&self) -> Vec<(ActionId, String)> {
        self.0.enabled_labeled()
    }
    pub fn label_of(&self, id: ActionId) -> Option<String> {
        self.0
            .enabled_labeled()
            .into_iter()
            .find(|(i, _)| *i == id)
            .map(|(_, l)| l)
    }
    pub fn state_debug(&self) -> String {
        self.0.state_debug()
    }
}

impl Clone for Erased {
    fn clone(&self) -> Self {
        Erased(self.0.box_clone())
    }
}

impl Sim for Erased {
    type Action = ErasedAction;
    fn enabled(&self) -> Vec<ErasedAction> {
        self.0.enabled_ids().into_iter().map(ErasedAction).collect()
    }
    fn action_id(action: &ErasedAction) -> ActionId {
        action.0
    }
    fn apply(&self, action: ErasedAction) -> Self {
        // Callers only pass actions obtained from `enabled()`; anything else is a bug.
        Erased(
            self.0
                .apply_id(action.0)
                .expect("apply() called with an action that is not enabled"),
        )
    }
    fn signature(&self) -> u64 {
        self.0.signature()
    }
    fn violated(&self) -> bool {
        self.0.violated()
    }
}

type Factory = Arc<dyn Fn() -> Erased + Send + Sync>;

/// Named simulation targets compiled into the binary.
#[derive(Clone, Default)]
pub struct Registry {
    entries: BTreeMap<String, (String, Factory)>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    pub fn new_with<S, F>(name: &str, description: &str, factory: F) -> Self
    where
        S: Sim + Debug + Send + Sync + 'static,
        F: Fn() -> S + Send + Sync + 'static,
    {
        Registry::new().with(name, description, factory)
    }

    /// Returns a new registry containing this one plus the new target.
    pub fn with<S, F>(self, name: &str, description: &str, factory: F) -> Self
    where
        S: Sim + Debug + Send + Sync + 'static,
        F: Fn() -> S + Send + Sync + 'static,
    {
        let mut entries = self.entries;
        let factory: Factory = Arc::new(move || Erased::new(factory()));
        entries.insert(name.to_string(), (description.to_string(), factory));
        Registry { entries }
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn description(&self, name: &str) -> Option<String> {
        self.entries.get(name).map(|(d, _)| d.clone())
    }

    pub fn instantiate(&self, name: &str) -> Option<Erased> {
        self.entries.get(name).map(|(_, f)| f())
    }
}
