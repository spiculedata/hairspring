//! What the engine can say about the inputs of a run before it starts: the built-in simulation backend knows its
//! executable and target; a target it cannot describe gives nothing, never a guess.
use hairspring_core::point::IdentityDraft;
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use std::sync::Arc;

fn engine() -> Engine {
    let registry = Arc::new(default_registry());
    Engine::new(registry.clone(), Config::from_registry(&registry))
}

#[test]
fn a_simulation_target_describes_its_executable_and_name() {
    let engine = engine();
    let name = engine.target_names().into_iter().next().expect("a target");
    let draft = engine.run_inputs(&name).expect("a sim target has inputs");
    assert!(matches!(draft, IdentityDraft::Sim(_)));
    let hashes = draft.input_hashes();
    assert_eq!(hashes.scheme, "sim/1");
    assert!(hashes.missing.is_empty(), "{:?}", hashes.missing);
    let names: Vec<&str> = hashes.fields.keys().map(String::as_str).collect();
    assert_eq!(names, ["backend", "hairspring_sha", "target"]);
}

#[test]
fn the_same_target_gives_the_same_hashes_and_another_target_another_name_hash() {
    let engine = engine();
    let mut names = engine.target_names();
    names.sort();
    assert!(names.len() >= 2);
    let first = engine.run_inputs(&names[0]).unwrap().input_hashes();
    let again = engine.run_inputs(&names[0]).unwrap().input_hashes();
    let other = engine.run_inputs(&names[1]).unwrap().input_hashes();
    assert_eq!(first, again);
    assert_ne!(first.fields["target"], other.fields["target"]);
    assert_eq!(
        first.fields["hairspring_sha"], other.fields["hairspring_sha"],
        "one executable"
    );
}

#[test]
fn an_unknown_target_has_no_inputs() {
    assert!(engine().run_inputs("no-such-target").is_none());
}
