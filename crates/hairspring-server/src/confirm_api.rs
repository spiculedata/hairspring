//! Starting a confirmation from the API (`POST /api/failures/{id}/confirm`) and asking how it is going
//! (`GET /api/failures/{id}/confirm`).
//!
//! It is off unless the server was started with a non-zero `max_confirmations`. A confirmation reruns and replays the
//! failure's target, up to the CLI's own default budget, on this server's engine and never on a remote worker, so its
//! results are recorded the way `hairspring confirm` records them: by this process, trusted. The caller chooses nothing
//! but the failure. At most one confirmation of a failure runs at a time, only `max_confirmations` run at once, and a
//! request beyond either is refused and never queued. What a request is refused for is said in fixed words.
use crate::api::blocking;
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_engine::confirm::{ConfirmRequest, DEFAULT_WALL_SECS};
use hairspring_mcp::{validate, ReplayPermit, ReplaySlots};
use hairspring_service::sanitize::public_text;
use hairspring_service::Role;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

/// Largest `max_confirmations` accepted. Each running confirmation holds a blocking thread and runs target code.
pub const MAX_MAX_CONFIRMATIONS: usize = 4;
/// What a refused confirmation is told to wait, in seconds.
const RETRY_AFTER_SECS: u64 = 30;
/// Failed attempts remembered so a page can say so; running ones are bounded by the cap, so this bounds the map.
const MAX_REMEMBERED_FAILURES: usize = 64;

const DISABLED_TEXT: &str = "confirmation from the browser is not enabled on this server";
const DISABLED_CODE: &str = "confirm_disabled";
const RUNNING_TEXT: &str = "a confirmation of this failure is already running";
const RUNNING_CODE: &str = "confirm_running";
const BUSY_TEXT: &str = "too many confirmations are running; try again shortly";
/// Said for an attempt that ended in an error. What went wrong is in the server's log, not in the answer.
const FAILED_TEXT: &str =
    "the confirmation could not run on this server; the server's log says why";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attempt {
    Running,
    Failed,
}

/// The confirmations this server runs: how many may run at once and which failures have one running now.
#[derive(Clone)]
pub struct Confirmations {
    max: usize,
    slots: ReplaySlots,
    attempts: Arc<Mutex<BTreeMap<String, Attempt>>>,
}

impl Confirmations {
    /// `max` is how many may run at once; 0 turns the feature off.
    pub fn new(max: usize) -> Confirmations {
        Confirmations {
            max,
            slots: ReplaySlots::new(max),
            attempts: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.max > 0
    }

    pub fn max(&self) -> usize {
        self.max
    }

    fn attempts(&self) -> MutexGuard<'_, BTreeMap<String, Attempt>> {
        // A panic while holding the lock leaves a map that is still valid.
        self.attempts.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the failure and a slot, or says why not: 409 when this failure already has one running, then 429 when
    /// every slot is taken. Nothing waits.
    fn begin(&self, failure_id: &str) -> Result<Running, ApiError> {
        let mut attempts = self.attempts();
        if attempts.get(failure_id) == Some(&Attempt::Running) {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                RUNNING_TEXT,
                RUNNING_CODE,
            ));
        }
        let permit = self
            .slots
            .try_acquire()
            .ok_or_else(|| ApiError::too_many_requests(BUSY_TEXT, RETRY_AFTER_SECS))?;
        attempts.insert(failure_id.to_owned(), Attempt::Running);
        Ok(Running {
            owner: self.clone(),
            failure_id: failure_id.to_owned(),
            succeeded: false,
            _permit: permit,
        })
    }

    fn state_of(&self, failure_id: &str) -> Option<Attempt> {
        self.attempts().get(failure_id).copied()
    }

    fn remember_failure(&self, failure_id: &str) {
        let mut attempts = self.attempts();
        let failed: Vec<String> = attempts
            .iter()
            .filter(|(_, a)| **a == Attempt::Failed)
            .map(|(id, _)| id.clone())
            .collect();
        if failed.len() >= MAX_REMEMBERED_FAILURES {
            for id in failed
                .iter()
                .take(failed.len() + 1 - MAX_REMEMBERED_FAILURES)
            {
                attempts.remove(id);
            }
        }
        attempts.insert(failure_id.to_owned(), Attempt::Failed);
    }

    fn forget(&self, failure_id: &str) {
        self.attempts().remove(failure_id);
    }
}

/// One confirmation in progress. Dropping it gives the slot back and ends the attempt: cleared when it succeeded,
/// remembered as failed when it did not (including a panic).
struct Running {
    owner: Confirmations,
    failure_id: String,
    succeeded: bool,
    _permit: ReplayPermit,
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.succeeded {
            self.owner.forget(&self.failure_id);
        } else {
            self.owner.remember_failure(&self.failure_id);
        }
    }
}

/// What `GET /api/me` adds for a caller who may start a confirmation: what one costs.
pub fn limits_for_me(confirms: &Confirmations) -> Value {
    json!({ "wall_secs": DEFAULT_WALL_SECS, "max_running": confirms.max() })
}

fn disabled() -> ApiError {
    ApiError::coded(StatusCode::FORBIDDEN, DISABLED_TEXT, DISABLED_CODE)
}

/// Starts a confirmation of a stored failure and answers `202` at once; the work goes on in the background, holding
/// its slot, and its checks are stored as they would be by the CLI. The order of the refusals is the order of what a
/// caller may learn: the role, then whether the server does this at all, then the id, the failure, its target, this
/// failure's own confirmation and last the server's slots.
pub async fn confirm(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    caller.need(Role::Run)?;
    if !s.confirms.enabled() {
        return Err(disabled());
    }
    if !validate::failure_id(&id) {
        return Err(ApiError::not_found());
    }
    let (svc, engine, failure) = (s.svc.clone(), s.engine.clone(), id.clone());
    blocking(move || svc.check_confirmable(&engine, &failure)).await?;
    let running = s.confirms.begin(&id)?;
    caller.log_action(&format!("confirmation of {id} started"));
    let who = caller.who();
    // Detached on purpose: the slot is held by `running` until the confirmation ends, even if the client goes away.
    drop(tokio::task::spawn_blocking(move || {
        run(&s, running, &id, &who);
    }));
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response())
}

/// Runs the confirmation with the CLI's defaults and stores it through the service. The server stopping ends it
/// between checks, and what ran is stored with the rest marked skipped.
fn run(s: &AppState, mut running: Running, id: &str, who: &str) {
    let request = ConfirmRequest::default();
    let result = s.svc.confirm_failure(&s.engine, id, &request, &mut |_| {
        !s.stopping.load(Ordering::SeqCst)
    });
    match result {
        Ok(checks) => {
            running.succeeded = true;
            eprintln!(
                "hairspring-server: confirmation of {id} by {who} {}",
                outcome_text(&checks)
            );
        }
        Err(e) => eprintln!(
            "hairspring-server: confirmation of {id} by {who} ended with an error: {}",
            public_text(&e.to_string())
        ),
    }
}

/// How the confirmation that has just ended went, in words for the log, from its own checks: "finished" when every
/// check ran, or why it stopped early (`cancelled`, `wall budget exhausted`). The failure's finding, which comes from
/// the newest confirmation that finished and so can be an earlier one, is given beside it and is not the outcome.
fn outcome_text(checks: &hairspring_service::FailureChecks) -> String {
    let newest = checks.checks.last().map(|c| c.confirmation_id.as_str());
    let own = || {
        checks
            .checks
            .iter()
            .filter(move |c| Some(c.confirmation_id.as_str()) == newest)
    };
    let stopped = own().find_map(|c| {
        c.guest
            .detail
            .get("stopped")
            .or_else(|| c.guest.detail.get("skipped"))
    });
    let finding = &checks.finding;
    match stopped {
        Some(why) => format!("stopped early: {why}; the failure's finding is {finding}"),
        None => format!("finished; the failure's finding is {finding}"),
    }
}

/// Whether a confirmation of the failure is running, as far as this process knows. The state is in memory, so a
/// restart forgets it. The checks themselves come from `GET /api/failures/{id}/checks`.
pub async fn status(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    if !validate::failure_id(&id) {
        return Err(ApiError::not_found());
    }
    let (svc, failure) = (s.svc.clone(), id.clone());
    let exists = blocking(move || svc.failure_exists(&failure)).await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    let mut body = json!({ "enabled": s.confirms.enabled() });
    match s.confirms.state_of(&id) {
        None => body["state"] = json!("idle"),
        Some(Attempt::Running) => body["state"] = json!("running"),
        Some(Attempt::Failed) => {
            body["state"] = json!("failed");
            body["reason"] = json!(FAILED_TEXT);
        }
    }
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "fail_00000000000000aa";
    const B: &str = "fail_00000000000000bb";

    #[test]
    fn a_cap_of_zero_is_off_and_hands_out_no_slot() {
        let confirms = Confirmations::new(0);

        assert!(!confirms.enabled());
        let refused = confirms.begin(A).err().expect("no slot");
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn the_same_failure_is_refused_with_409_before_the_slots_are_counted() {
        let confirms = Confirmations::new(1);
        let _first = confirms.begin(A).expect("a free slot");

        let same = confirms.begin(A).err().expect("refused");
        let other = confirms.begin(B).err().expect("refused");

        assert_eq!(same.status, StatusCode::CONFLICT);
        assert_eq!(other.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn ending_gives_the_slot_back_and_a_failure_is_remembered_until_the_next_attempt() {
        let confirms = Confirmations::new(1);

        let mut ok = confirms.begin(A).expect("a free slot");
        assert!(confirms.state_of(A) == Some(Attempt::Running));
        ok.succeeded = true;
        drop(ok);
        assert!(confirms.state_of(A).is_none());

        drop(confirms.begin(B).expect("the slot came back"));
        assert!(confirms.state_of(B) == Some(Attempt::Failed));
        let retry = confirms
            .begin(B)
            .expect("a failed failure may be tried again");
        assert!(confirms.state_of(B) == Some(Attempt::Running));
        drop(retry);
    }

    #[test]
    fn the_failures_remembered_are_bounded() {
        let confirms = Confirmations::new(1);

        for n in 0..(MAX_REMEMBERED_FAILURES * 2) {
            drop(confirms.begin(&format!("fail_{n:016x}")).expect("free"));
        }

        assert_eq!(confirms.attempts().len(), MAX_REMEMBERED_FAILURES);
    }

    fn view(
        confirmation: &str,
        status: &str,
        detail: &[(&str, &str)],
    ) -> hairspring_service::CheckView {
        hairspring_service::CheckView {
            id: "chk".into(),
            confirmation_id: confirmation.into(),
            ordinal: 0,
            kind: "same_seed_rerun".into(),
            status: status.into(),
            counts: hairspring_engine::confirm::CheckCounts {
                passed: 0,
                failed: 0,
                inconclusive: 0,
            },
            artefacts: vec![],
            environment: Default::default(),
            started_at: 0,
            finished_at: 0,
            runner_trusted: true,
            created_at: 0,
            guest: hairspring_service::CheckGuest {
                command: String::new(),
                detail: detail
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
        }
    }

    fn checks(
        finding: &str,
        list: Vec<hairspring_service::CheckView>,
    ) -> hairspring_service::FailureChecks {
        hairspring_service::FailureChecks {
            failure_id: A.into(),
            finding: finding.into(),
            confirmation_id: Some("conf_old".into()),
            environment_pinned: false,
            checks: list,
        }
    }

    #[test]
    fn a_cancelled_confirmation_is_logged_as_stopped_not_as_the_finding_of_an_earlier_one() {
        let list = vec![
            view("conf_old", "pass", &[]),
            view("conf_new", "inconclusive", &[("stopped", "cancelled")]),
            view("conf_new", "skipped", &[("skipped", "cancelled")]),
        ];

        let text = outcome_text(&checks("confirmed", list));

        assert_eq!(
            text,
            "stopped early: cancelled; the failure's finding is confirmed"
        );
    }

    #[test]
    fn a_confirmation_that_ran_every_check_is_logged_as_finished() {
        let list = vec![view("conf_new", "pass", &[]), view("conf_new", "pass", &[])];

        let text = outcome_text(&checks("confirmed", list));

        assert_eq!(text, "finished; the failure's finding is confirmed");
    }

    #[test]
    fn an_earlier_confirmations_stop_is_not_attributed_to_the_newest() {
        let list = vec![
            view("conf_old", "skipped", &[("skipped", "cancelled")]),
            view("conf_new", "pass", &[]),
        ];

        assert!(outcome_text(&checks("rerun_repeated", list)).starts_with("finished;"));
    }
}
