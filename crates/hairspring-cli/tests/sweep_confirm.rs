//! `hairspring test --sweep --confirm`: after the sweep, the confirmation pipeline runs on each stored failure row.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";
const PROJECT: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"guest.sh\"]\n[target.svc.budget]\nmax_trials = 20\n";
/// Seeds 3 and 8 break property A, seeds 4 and 9 break property B, the rest pass.
const GUEST: &str = r#"ev() { echo "HAIRSPRING_EVENT $1"; }
ev '{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}'
ev '{"v":1,"kind":"catalog","proc":"p1","id":"prop.a","type":"always","must_hit":true,"msg":"a"}'
ev '{"v":1,"kind":"catalog","proc":"p1","id":"prop.b","type":"always","must_hit":true,"msg":"b"}'
m=$((FAKE_SCHED_SEED % 5)); fa=0; fb=0
[ $m -eq 3 ] && fa=1
[ $m -eq 4 ] && fb=1
ev "{\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"prop.a\",\"hits\":1,\"trues\":$((1-fa)),\"falses\":$fa,\"first_false\":null,\"first_true\":null}"
ev "{\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"prop.b\",\"hits\":1,\"trues\":$((1-fb)),\"falses\":$fb,\"first_false\":null,\"first_true\":null}"
ev '{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":2,"evals":2,"dropped":0,"threads":0}'
[ $((fa+fb)) -gt 0 ] && { echo 'HAIRSPRING_VIOLATION: broken'; exit 1; }
exit 0
"#;

fn project(name: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!(
        "hairspring-sweep-confirm-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("hairspring.toml"), PROJECT).unwrap();
    std::fs::write(dir.join("guest.sh"), GUEST).unwrap();
    (dir, hermit)
}

fn run(dir: &Path, hermit: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .output()
        .expect("binary runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Seeds 3 and 8 break one property and seeds 4 and 9 another, so the sweep stores a failure row for each property.
const SWEEP: &[&str] = &["test", "svc", "--sweep", "--seeds", "1..11"];

fn with<'a>(more: &[&'a str]) -> Vec<&'a str> {
    SWEEP.iter().chain(more).copied().collect()
}

/// How many checks each failure row of the newest run holds.
fn stored_checks(dir: &Path) -> Vec<usize> {
    use hairspring_store::{SqliteStore, Store};
    let store = SqliteStore::open(&dir.join(".hairspring/hairspring.db")).unwrap();
    let run_id = store.list_runs(1).unwrap().remove(0).id;
    store
        .list_failures(&run_id)
        .unwrap()
        .iter()
        .map(|f| store.list_failure_checks(&f.id).unwrap().len())
        .collect()
}

#[test]
fn a_sweep_without_the_flag_confirms_nothing() {
    let (dir, hermit) = project("off");
    let o = run(&dir, &hermit, SWEEP);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert_eq!(stored_checks(&dir), vec![0, 0]);
}

#[test]
fn the_flag_confirms_every_stored_failure_row_after_the_sweep() {
    let (dir, hermit) = project("on");
    let o = run(&dir, &hermit, &with(&["--confirm"]));
    let said = text(&o);
    assert_eq!(o.status.code(), Some(1), "{said}");
    assert_eq!(said.matches("finding: confirmed").count(), 2, "{said}");
    assert_eq!(said.matches("failure fail_").count(), 2, "{said}");
    let sweep_at = said.find("failing seeds").expect("the sweep report");
    assert!(said.find("failure fail_").unwrap() > sweep_at, "{said}");
    let checks = stored_checks(&dir);
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().all(|n| *n > 0), "{checks:?}");
}

#[test]
fn a_sweep_that_finds_nothing_has_nothing_to_confirm() {
    let (dir, hermit) = project("clean");
    let o = run(
        &dir,
        &hermit,
        &["test", "svc", "--sweep", "--seeds", "1..3", "--confirm"],
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(!text(&o).contains("failure fail_"), "{}", text(&o));
}

#[test]
fn the_flag_needs_a_sweep() {
    let (dir, hermit) = project("nosweep");
    let o = run(&dir, &hermit, &["test", "svc", "--seed", "1", "--confirm"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("--sweep"), "{}", text(&o));
}

#[test]
fn the_flag_needs_text_output_so_the_report_stays_one_document() {
    let (dir, hermit) = project("json");
    let o = run(&dir, &hermit, &with(&["--confirm", "--format", "json"]));
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("--format text"), "{}", text(&o));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "",
        "nothing was run or printed"
    );
}

#[test]
fn a_sim_target_is_skipped_with_a_note_and_the_later_targets_still_run() {
    let (dir, hermit) = project("sim");
    let sim = "[target.aaa-sim]\nkind = \"sim\"\nsim = \"lock-lease\"\n";
    let toml = std::fs::read_to_string(dir.join("hairspring.toml")).unwrap();
    std::fs::write(dir.join("hairspring.toml"), format!("{sim}{toml}")).unwrap();
    let o = run(
        &dir,
        &hermit,
        &[
            "test",
            "--sweep",
            "--trials",
            "30",
            "--seed",
            "1",
            "--confirm",
        ],
    );
    let said = text(&o);
    assert_eq!(o.status.code(), Some(1), "the sweep's own status: {said}");
    assert!(said.contains("sweeping svc"), "later target ran: {said}");
    assert!(said.contains("aaa-sim"), "{said}");
    assert!(
        said.contains("--confirm: skipped, a sim target cannot be confirmed"),
        "{said}"
    );
    assert!(!said.contains("confirming its failures failed"), "{said}");
    assert!(
        said.contains("finding: confirmed"),
        "svc was confirmed: {said}"
    );
}

#[test]
fn a_confirm_error_on_a_process_target_is_reported_and_the_later_targets_still_run() {
    let (dir, hermit) = project("confirm-error");
    // `aaa-logged` sorts first and sweeps normally, but its Hermit logging makes the confirmation pipeline refuse it.
    let first = "[target.aaa-logged]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"guest.sh\"]\nhermit_args = [\"--verbose\"]\n[target.aaa-logged.budget]\nmax_trials = 20\n";
    let toml = std::fs::read_to_string(dir.join("hairspring.toml")).unwrap();
    std::fs::write(dir.join("hairspring.toml"), format!("{first}{toml}")).unwrap();
    let o = run(
        &dir,
        &hermit,
        &["test", "--sweep", "--seeds", "1..11", "--confirm"],
    );
    let said = text(&o);
    assert_eq!(
        o.status.code(),
        Some(2),
        "the command ends with the error status: {said}"
    );
    assert!(
        said.contains("error: aaa-logged: the sweep finished and is stored as"),
        "the error names the target and says the sweep is stored: {said}"
    );
    assert!(said.contains("sweeping svc"), "later target ran: {said}");
    assert!(
        said.contains("finding: confirmed"),
        "svc was confirmed: {said}"
    );
}
