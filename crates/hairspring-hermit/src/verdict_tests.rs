//! The order in which `classify` weighs a trial's evidence.
use crate::events::EventsFault;
use crate::exec::Trial;
use crate::trace::{classify, Verdict};
use hairspring_props::TrialEvents;

const REPORT_FALSE: &str = concat!(
    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"proc\":\"p1\"}\n",
    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"x.id\",\"type\":\"always\",\"must_hit\":true}\n",
    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"x.id\",\"hits\":2,\"trues\":1,\"falses\":1,\"first_false\":{\"thread\":\"t\",\"ord\":4,\"vals\":[0,0,0,0]},\"first_true\":null}\n",
    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\",\"status\":0,\"sites\":1,\"evals\":2,\"dropped\":0,\"threads\":0}\n",
);

fn with_false() -> Trial {
    Trial {
        exit_code: Some(0),
        events: Some(TrialEvents::from_output(REPORT_FALSE.as_bytes()).fold()),
        ..Trial::default()
    }
}

fn with_fault(fault: EventsFault, base: Trial) -> Trial {
    Trial {
        events_error: Some(fault),
        ..base
    }
}

#[test]
fn a_false_property_is_a_violation_naming_the_id_thread_and_ordinal() {
    match classify(&with_false(), 5) {
        Verdict::Violation(m) => {
            for part in ["x.id", "thread t", "ordinal 4"] {
                assert!(m.contains(part), "{part} in {m}");
            }
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_config_fault_is_a_run_error_before_anything_else() {
    let t = with_fault(EventsFault::Config("bad".into()), with_false());
    assert_eq!(classify(&t, 5), Verdict::Infra("bad".into()));
}

#[test]
fn an_infrastructure_fault_makes_the_trial_inconclusive_even_with_a_false_read() {
    let t = with_fault(EventsFault::Infra("disk".into()), with_false());
    assert!(matches!(classify(&t, 5), Verdict::Inconclusive(m) if m.contains("disk")));
}

#[test]
fn a_hang_outranks_an_infrastructure_fault() {
    let t = Trial {
        timed_out: true,
        ..with_fault(EventsFault::Infra("disk".into()), Trial::default())
    };
    assert!(matches!(classify(&t, 5), Verdict::Hang(_)));
}

#[test]
fn a_false_read_before_a_guest_fault_still_fails_the_trial() {
    let t = with_fault(EventsFault::Guest("cut".into()), with_false());
    assert!(matches!(classify(&t, 5), Verdict::Violation(_)));
}

#[test]
fn a_clean_exit_with_a_guest_fault_is_inconclusive_not_a_pass() {
    let t = with_fault(
        EventsFault::Guest("cut".into()),
        Trial {
            exit_code: Some(0),
            ..Trial::default()
        },
    );
    assert!(matches!(classify(&t, 5), Verdict::Inconclusive(_)));
}

#[test]
fn exit_status_still_decides_when_no_property_failed() {
    let t = Trial {
        exit_code: Some(4),
        ..Trial::default()
    };
    assert_eq!(classify(&t, 5), Verdict::Violation("exit code 4".into()));
}

#[test]
fn a_property_failure_outranks_the_legacy_violation_line_and_the_exit_status() {
    let t = Trial {
        exit_code: Some(9),
        stdout: "HAIRSPRING_VIOLATION: old\n".into(),
        ..with_false()
    };
    assert!(matches!(classify(&t, 5), Verdict::Violation(m) if m.contains("x.id")));
}
