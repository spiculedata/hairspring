//! `Backend::probe` and `Engine::probe_backend`: a probe answers for the machine, by backend id, and never panics.
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::probe::{ProbeReport, ProbeRow, ProbeStatus};
use hairspring_engine::report::{ReplayOutcome, RunOutcome};
use hairspring_engine::EngineError;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Behaviour {
    Default,
    Missing,
    Panics,
}

struct Fake {
    id: &'static str,
    behaviour: Behaviour,
}

impl Backend for Fake {
    fn id(&self) -> &'static str {
        self.id
    }
    fn supports(&self, _: &TargetKind) -> bool {
        false
    }
    fn run(
        &self,
        _: &TargetSpec,
        _: &RunRequest,
        _: Budget,
        _: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        unreachable!("not used")
    }
    fn replay(&self, _: &TargetSpec, _: &Bundle) -> Result<ReplayOutcome, EngineError> {
        unreachable!("not used")
    }
    fn probe(&self) -> ProbeReport {
        match self.behaviour {
            Behaviour::Default => ProbeReport::nothing_to_check(self.display_name()),
            Behaviour::Missing => ProbeReport {
                subject: "fake backend".into(),
                rows: vec![ProbeRow {
                    label: "tool".into(),
                    status: ProbeStatus::Fail,
                    detail: "not found".into(),
                }],
                ready_note: String::new(),
                missing: vec!["tool was not found: install it".into()],
            },
            Behaviour::Panics => panic!("a probe that goes wrong"),
        }
    }
}

fn engine_with(backends: &[(&'static str, Behaviour)]) -> Engine {
    let mut engine = Engine::new(Arc::new(Registry::new()), Config::default());
    for (id, behaviour) in backends {
        engine = engine.with_backend(Arc::new(Fake {
            id,
            behaviour: *behaviour,
        }));
    }
    engine
}

#[test]
fn the_simulation_backend_has_nothing_to_check_and_is_ready() {
    let engine = engine_with(&[]);
    let report = engine.probe_backend("sim").expect("sim is registered");
    assert!(report.is_ready());
    assert!(report.rows.is_empty());
}

#[test]
fn a_backend_that_does_not_override_probe_gets_the_default() {
    let engine = engine_with(&[("fake", Behaviour::Default)]);
    let report = engine.probe_backend("fake").unwrap();
    assert!(report.is_ready());
}

#[test]
fn a_probe_that_finds_something_missing_is_returned_as_it_is() {
    let engine = engine_with(&[("fake", Behaviour::Missing)]);
    let report = engine.probe_backend("fake").unwrap();
    assert!(!report.is_ready());
    assert_eq!(report.missing, ["tool was not found: install it"]);
}

#[test]
fn a_probe_that_panics_becomes_a_report_that_is_not_ready() {
    let engine = engine_with(&[("fake", Behaviour::Panics)]);
    let report = engine.probe_backend("fake").unwrap();
    assert!(!report.is_ready());
    assert_eq!(report.subject, "fake");
    assert!(report.missing[0].contains("failed unexpectedly"));
}

#[test]
fn an_unknown_backend_is_an_error_listing_the_ones_there_are() {
    let engine = engine_with(&[("fake", Behaviour::Default)]);
    let err = engine.probe_backend("nope").unwrap_err();
    assert_eq!(
        err,
        EngineError::InvalidRequest("unknown backend `nope`; available: sim, fake".into())
    );
}

#[test]
fn a_backend_name_that_is_long_or_has_control_characters_is_shortened_and_escaped() {
    let engine = engine_with(&[]);
    let long = format!("{}\n", "x".repeat(500));
    let text = engine.probe_backend(&long).unwrap_err().to_string();
    assert!(!text.contains('\n'), "{text}");
    assert!(text.len() < 200, "{text}");
}

#[test]
fn an_id_that_two_backends_claim_is_refused_rather_than_guessed() {
    let engine = engine_with(&[("fake", Behaviour::Default), ("fake", Behaviour::Missing)]);
    let err = engine.probe_backend("fake").unwrap_err();
    assert!(matches!(err, EngineError::BackendChoice(_)), "{err:?}");
}

#[test]
fn backend_ids_are_listed_in_registration_order() {
    let engine = engine_with(&[("fake", Behaviour::Default)]);
    assert_eq!(engine.backend_ids(), ["sim", "fake"]);
}
