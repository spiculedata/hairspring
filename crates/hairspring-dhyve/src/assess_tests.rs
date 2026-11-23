use crate::assess::assess;
use crate::facts::DhyveFacts;
use crate::facts::{BinaryFact, CpuFact, ModuleFact};
use hairspring_engine::probe::ProbeReport;
use hairspring_engine::probe::ProbeStatus;

fn good() -> DhyveFacts {
    DhyveFacts {
        os: "freebsd".into(),
        arch: "x86_64".into(),
        os_release: Some("15.0-RELEASE".into()),
        cpu: CpuFact::Ept,
        module: ModuleFact::Loaded,
        bhyve: BinaryFact::Found {
            path: "/opt/d/bhyve".into(),
            sha256: Some("ab12".into()),
        },
        bhyvectl: BinaryFact::Found {
            path: "/opt/d/bhyvectl".into(),
            sha256: Some("cd34".into()),
        },
        leftover_vms: Some(vec![]),
        panic_on_nmi: Some(0),
    }
}

fn text(report: &ProbeReport) -> String {
    let mut out = Vec::new();
    report.write(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn status_of(report: &ProbeReport, label: &str) -> ProbeStatus {
    report
        .rows
        .iter()
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("no row {label}: {}", text(report)))
        .status
}

#[test]
fn a_host_with_everything_in_place_is_ready_and_claims_nothing_more() {
    let report = assess(&good());
    assert!(report.is_ready(), "{}", text(&report));
    assert!(report.ready_note.contains("experimental"));
    assert!(report.ready_note.contains("unproven"));
    assert!(text(&report).contains("result: ok:"));
}

#[test]
fn a_host_that_is_not_freebsd_gets_one_row_and_one_missing_entry() {
    let report = assess(&DhyveFacts::platform_only("linux", "aarch64"));
    assert!(!report.is_ready());
    assert_eq!(report.rows.len(), 1, "{}", text(&report));
    assert_eq!(report.rows[0].label, "os");
    assert_eq!(report.rows[0].status, ProbeStatus::Fail);
    assert_eq!(report.rows[0].detail, "linux/aarch64");
    assert_eq!(report.missing.len(), 1);
    let m = &report.missing[0];
    assert!(m.contains("FreeBSD 15 on x86-64"), "{m}");
    assert!(m.contains("`dhyve` feature"), "{m}");
    assert!(m.contains("This machine is linux/aarch64."), "{m}");
}

#[test]
fn freebsd_on_another_architecture_is_refused_the_same_way() {
    let facts = DhyveFacts {
        arch: "aarch64".into(),
        ..good()
    };
    let report = assess(&facts);
    assert!(!report.is_ready());
    assert_eq!(report.rows.len(), 1);
    assert!(report.missing[0].contains("This machine is freebsd/aarch64."));
}

#[test]
fn the_release_is_shown_and_never_judged() {
    for release in ["14.2-RELEASE", "15.0-RELEASE", "15.1-RELEASE"] {
        let facts = DhyveFacts {
            os_release: Some(release.into()),
            ..good()
        };
        let report = assess(&facts);
        assert!(report.is_ready(), "{release}");
        assert_eq!(status_of(&report, "os"), ProbeStatus::Ok);
        assert!(report.rows[0].detail.contains(release));
    }
}

#[test]
fn an_unreadable_release_is_a_note() {
    let facts = DhyveFacts {
        os_release: None,
        ..good()
    };
    let report = assess(&facts);
    assert!(report.is_ready());
    assert_eq!(status_of(&report, "os"), ProbeStatus::Note);
}

#[test]
fn a_cpu_without_ept_blocks_and_an_unreadable_one_only_notes() {
    let no_ept = assess(&DhyveFacts {
        cpu: CpuFact::NoEpt,
        ..good()
    });
    assert!(!no_ept.is_ready());
    assert_eq!(status_of(&no_ept, "cpu"), ProbeStatus::Fail);
    assert!(no_ept.missing[0].contains("VT-x with EPT not found"));

    let unknown = assess(&DhyveFacts {
        cpu: CpuFact::Unknown,
        ..good()
    });
    assert!(unknown.is_ready());
    assert_eq!(status_of(&unknown, "cpu"), ProbeStatus::Note);
}

#[test]
fn a_missing_module_says_an_operator_loads_it_and_hairspring_never_does() {
    let report = assess(&DhyveFacts {
        module: ModuleFact::NotLoaded,
        ..good()
    });
    assert!(!report.is_ready());
    assert_eq!(status_of(&report, "module"), ProbeStatus::Fail);
    let m = &report.missing[0];
    assert!(m.contains("no vmm module is loaded"), "{m}");
    assert!(m.contains("Hairspring never loads one"), "{m}");
}

#[test]
fn an_unanswered_module_question_is_a_failure_not_a_pass() {
    let report = assess(&DhyveFacts {
        module: ModuleFact::Unanswered("kldstat did not answer within 5 s".into()),
        ..good()
    });
    assert!(!report.is_ready());
    assert!(report.missing[0].contains("kldstat did not answer"));
}

#[test]
fn a_loaded_module_gets_a_note_that_its_kind_is_not_checked() {
    let report = assess(&good());
    assert_eq!(status_of(&report, "module kind"), ProbeStatus::Note);
    assert!(report
        .rows
        .iter()
        .any(|r| r.label == "module kind" && r.detail.contains("not checked")));
}

#[test]
fn each_program_is_shown_with_its_hash_and_blocks_when_missing() {
    let ok = assess(&good());
    let bhyve = ok.rows.iter().find(|r| r.label == "bhyve").unwrap();
    assert_eq!(bhyve.status, ProbeStatus::Ok);
    assert!(bhyve.detail.contains("/opt/d/bhyve"));
    assert!(bhyve.detail.contains("sha256 ab12"));

    let report = assess(&DhyveFacts {
        bhyve: BinaryFact::NotConfigured,
        bhyvectl: BinaryFact::Unusable {
            path: "/nope/bhyvectl".into(),
        },
        ..good()
    });
    assert_eq!(report.missing.len(), 2, "{}", text(&report));
    assert!(report.missing[0].contains("HAIRSPRING_DHYVE_BHYVE"));
    assert!(report.missing[1].contains("/nope/bhyvectl"));
    assert!(report.missing[1].contains("HAIRSPRING_DHYVE_BHYVECTL"));
}

#[test]
fn an_unreadable_hash_is_said_not_hidden() {
    let report = assess(&DhyveFacts {
        bhyve: BinaryFact::Found {
            path: "/b".into(),
            sha256: None,
        },
        ..good()
    });
    let row = report.rows.iter().find(|r| r.label == "bhyve").unwrap();
    assert!(row.detail.contains("sha256 unreadable"), "{}", row.detail);
}

#[test]
fn a_leftover_vm_blocks_and_names_the_destroy_command() {
    let report = assess(&DhyveFacts {
        leftover_vms: Some(vec!["hs_1_2_3".into()]),
        ..good()
    });
    assert!(!report.is_ready());
    let m = &report.missing[0];
    assert!(m.contains("bhyvectl --destroy --vm=hs_1_2_3"), "{m}");
}

#[test]
fn an_unreadable_vm_list_is_a_note() {
    let report = assess(&DhyveFacts {
        leftover_vms: None,
        ..good()
    });
    assert!(report.is_ready());
    assert_eq!(status_of(&report, "leftovers"), ProbeStatus::Note);
}

#[test]
fn a_non_zero_nmi_setting_is_a_note_about_stray_nmis() {
    let report = assess(&DhyveFacts {
        panic_on_nmi: Some(255),
        ..good()
    });
    assert!(report.is_ready());
    assert_eq!(status_of(&report, "nmi"), ProbeStatus::Note);
    let row = report.rows.iter().find(|r| r.label == "nmi").unwrap();
    assert!(row.detail.contains("may panic this host"), "{}", row.detail);
}

#[test]
fn a_zero_or_unreadable_nmi_setting_is_not_a_warning() {
    for value in [Some(0), None] {
        let report = assess(&DhyveFacts {
            panic_on_nmi: value,
            ..good()
        });
        let row = report.rows.iter().find(|r| r.label == "nmi").unwrap();
        assert!(!row.detail.contains("panic this host"), "{}", row.detail);
    }
}

#[test]
fn the_report_carries_no_history_of_earlier_use() {
    let all = every_text().to_lowercase();
    for word in ["has been used", "reboot", "lane", "window", "box"] {
        assert!(!all.contains(word), "{word} in {all}");
    }
}

fn every_text() -> String {
    [
        text(&assess(&good())),
        text(&assess(&DhyveFacts {
            module: ModuleFact::NotLoaded,
            cpu: CpuFact::NoEpt,
            leftover_vms: Some(vec!["hs_1".into()]),
            panic_on_nmi: Some(255),
            os_release: None,
            ..good()
        })),
        text(&assess(&DhyveFacts::platform_only("macos", "aarch64"))),
    ]
    .join("\n")
}

#[test]
fn the_report_never_uses_wording_the_backend_has_not_earned() {
    let all = every_text().to_lowercase();
    for word in ["deterministic", "reproduced", "proves", "fork"] {
        assert!(!all.contains(word), "{word} in {all}");
    }
}
