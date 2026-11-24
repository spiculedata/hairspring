//! The control endpoint against a hand-driven peer that plays the VM program's part.
use super::*;
use crate::runlog::{connect_text, stop_text};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicU32;
use std::sync::Mutex;

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new() -> Env {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hsdt-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Env { dir }
    }

    fn sock(&self) -> PathBuf {
        self.dir.join("c.sock")
    }

    fn log(&self) -> PathBuf {
        self.dir.join("run.log")
    }

    fn write_log(&self, lines: &[String]) {
        let text: String = lines
            .iter()
            .enumerate()
            .map(|(i, l)| format!("[det_counter={}] {l}\n", i + 1))
            .collect();
        std::fs::write(self.log(), text).unwrap();
    }

    fn spec(&self, mode: Mode, rip: Option<u64>) -> DirectorSpec {
        DirectorSpec {
            sock_path: self.sock(),
            mode,
            run_log: self.log(),
            restore: (mode == Mode::Restore).then(|| RestoreScanKeys {
                stop_seq: 3,
                branch_count: 1000,
                vtime_ns: 5,
                vcpu_id: 2,
                rip,
                sock: self.sock().display().to_string(),
            }),
            connect_wait: Duration::from_millis(400),
            resume_limit: Duration::from_millis(300),
            resume_poll: Duration::from_millis(5),
            shutdown_delay: Duration::from_millis(10),
        }
    }
}

fn ready() -> Message {
    Message {
        kind: Kind::Ready.code(),
        stop_seq: 3,
        branch_count: 1000,
        vtime_ns: 5,
        arg0: 9,
        vcpu_id: 2,
        ..Message::default()
    }
}

fn connect(env: &Env) -> UnixStream {
    for _ in 0..200 {
        if let Ok(s) = UnixStream::connect(env.sock()) {
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            return s;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("the endpoint never listened");
}

fn send(peer: &mut UnixStream, m: &Message) {
    peer.write_all(&m.encode()).unwrap();
}

/// What the peer read back: `None` at end of stream.
fn answer(peer: &mut UnixStream) -> Option<Message> {
    let mut buf = [0u8; SIZE];
    peer.read_exact(&mut buf).ok()?;
    Some(Message::decode(&buf).unwrap())
}

fn no_callback() -> OnReady {
    Box::new(|_| Ok(()))
}

fn stop_line() -> String {
    stop_text(3, 1000)
}

#[test]
fn a_make_run_calls_back_checks_the_hold_and_sends_a_shutdown_that_echoes_the_stop() {
    let env = Env::new();
    env.write_log(&[stop_line()]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let director = Director::start(
        env.spec(Mode::Make, None),
        Box::new(move |e| {
            log.lock().unwrap().push(e.stop_seq);
            Ok(())
        }),
    )
    .unwrap();
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    let shutdown = answer(&mut peer).unwrap();
    assert_eq!(shutdown.kind(), Some(Kind::Shutdown));
    assert_eq!((shutdown.stop_seq, shutdown.vcpu_id), (3, 2));
    let report = director.finish(&env.sock());
    assert_eq!(*seen.lock().unwrap(), vec![3]);
    assert!(report.shutdown_sent && report.error.is_none());
    assert_eq!(report.close, Some(CloseWhy::Delay));
    assert_eq!(report.held, Some(true));
    assert_eq!(report.ready_event, Some(ready()));
    assert!(!env.sock().exists());
}

#[test]
fn a_callback_that_says_no_means_no_shutdown() {
    let env = Env::new();
    env.write_log(&[stop_line()]);
    let director = Director::start(
        env.spec(Mode::Make, None),
        Box::new(|_| Err("files not settled".to_string())),
    )
    .unwrap();
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    assert!(answer(&mut peer).is_none());
    let report = director.finish(&env.sock());
    assert!(!report.shutdown_sent);
    assert_eq!(report.close, Some(CloseWhy::Incomplete));
    assert_eq!(
        report.callback_failure.as_deref(),
        Some("files not settled")
    );
}

#[test]
fn a_stop_that_is_no_longer_held_gets_no_shutdown() {
    let env = Env::new();
    env.write_log(&[stop_line(), "VMEXIT reason=HLT".to_string()]);
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    assert!(answer(&mut peer).is_none());
    let report = director.finish(&env.sock());
    assert_eq!(report.held, Some(false));
    assert_eq!(report.close, Some(CloseWhy::NotHeld));
    assert!(!report.shutdown_sent);
}

#[test]
fn a_restore_is_resumed_with_the_stop_echoed_and_closed_once_the_connect_line_shows() {
    let env = Env::new();
    let sock = env.sock().display().to_string();
    env.write_log(&[
        stop_line(),
        "VMEXIT reason=DEBUG(21) rip=0xabc".to_string(),
        "VTIME 0 -> 5 (+1000 branches)".to_string(),
        connect_text(&sock),
    ]);
    let director = Director::start(env.spec(Mode::Restore, Some(0xabc)), no_callback()).unwrap();
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    let resume = answer(&mut peer).unwrap();
    assert_eq!(resume.kind(), Some(Kind::Resume));
    assert_eq!((resume.stop_seq, resume.vcpu_id), (3, 2));
    assert_eq!(resume.branch_count, 4_000_000_000);
    assert!(answer(&mut peer).is_none(), "the endpoint closes");
    let report = director.finish(&env.sock());
    assert_eq!(report.resume, ResumeResult::Seen);
    assert_eq!(report.close, Some(CloseWhy::Seen));
    assert_eq!(report.skipped, 2);
}

fn restore_report(lines: &[String], rip: Option<u64>, exited: bool) -> Report {
    let env = Env::new();
    env.write_log(lines);
    let director = Director::start(env.spec(Mode::Restore, rip), no_callback()).unwrap();
    if exited {
        director.vm_exited();
    }
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    let _ = answer(&mut peer);
    director.finish(&env.sock())
}

#[test]
fn a_stale_resume_line_is_not_a_resume_taken() {
    let r = restore_report(
        &[stop_line(), "director stale resume stop_seq=2".into()],
        None,
        false,
    );
    assert_eq!(
        r.resume,
        ResumeResult::Unseen("the VM refused the command".to_string())
    );
    assert!(r.first_line.as_deref().unwrap().contains("stale"));
    assert_eq!(r.close, Some(CloseWhy::Excluded));
}

#[test]
fn another_line_first_is_not_a_resume_taken() {
    let r = restore_report(&[stop_line(), "VMEXIT reason=HLT".into()], None, false);
    assert_eq!(
        r.resume,
        ResumeResult::Unseen("another line came first".to_string())
    );
    assert!(r.first_line.as_deref().unwrap().contains("HLT"));
    assert_eq!(r.close, Some(CloseWhy::Other));
}

#[test]
fn an_unknown_restored_pointer_means_a_debug_line_decides_the_state() {
    let sock = Env::new().sock().display().to_string();
    let lines = [
        stop_line(),
        "VMEXIT reason=DEBUG(21) rip=0xabc".to_string(),
        connect_text(&sock),
    ];
    let r = restore_report(&lines, None, false);
    assert_eq!(r.close, Some(CloseWhy::Other));
}

#[test]
fn no_line_by_the_limit_is_a_limit_close_and_an_exited_vm_is_an_exited_close() {
    let r = restore_report(&[stop_line()], None, false);
    assert_eq!(r.close, Some(CloseWhy::Limit));
    assert!(matches!(r.resume, ResumeResult::Unseen(ref t) if t == "limit"));
    let r = restore_report(&[stop_line()], None, true);
    assert_eq!(r.close, Some(CloseWhy::Exited));
}

#[test]
fn a_stopped_event_is_answered_with_a_shutdown_and_counted() {
    let env = Env::new();
    env.write_log(&[stop_line()]);
    let director = Director::start(env.spec(Mode::Restore, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    send(
        &mut peer,
        &Message {
            kind: Kind::Stopped.code(),
            stop_seq: 8,
            vcpu_id: 1,
            ..Message::default()
        },
    );
    let shutdown = answer(&mut peer).unwrap();
    assert_eq!(
        (shutdown.kind(), shutdown.stop_seq, shutdown.vcpu_id),
        (Some(Kind::Shutdown), 8, 1)
    );
    let report = director.finish(&env.sock());
    assert_eq!(report.stopped, 1);
    assert!(report.shutdown_sent);
}

#[test]
fn a_vm_that_never_connects_ends_the_endpoint_with_no_connect() {
    let env = Env::new();
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    let report = director.finish(&env.sock());
    assert_eq!(report.error, Some(ControlError::NoConnect));
    assert!(!report.connected);
}

#[test]
fn a_dropped_socket_is_a_lost_connection() {
    let env = Env::new();
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    drop(connect(&env));
    let report = director.finish(&env.sock());
    assert!(
        matches!(report.error, Some(ControlError::Lost(_))),
        "{:?}",
        report.error
    );
    assert!(report.connected);
}

#[test]
fn a_message_cut_short_and_a_message_of_another_protocol_are_errors() {
    let env = Env::new();
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    peer.write_all(&ready().encode()[..30]).unwrap();
    drop(peer);
    assert!(matches!(
        director.finish(&env.sock()).error,
        Some(ControlError::Lost(_))
    ));

    let env = Env::new();
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    let mut raw = ready().encode();
    raw[0] ^= 0xff;
    peer.write_all(&raw).unwrap();
    assert_eq!(
        director.finish(&env.sock()).error,
        Some(ControlError::Wire(WireError::Protocol))
    );
}

#[test]
fn an_unknown_message_is_ignored_and_a_cancel_ends_the_wait() {
    let env = Env::new();
    env.write_log(&[stop_line()]);
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    send(
        &mut peer,
        &Message {
            kind: 99,
            ..Message::default()
        },
    );
    thread::sleep(Duration::from_millis(60));
    assert!(!director.is_finished());
    director.cancel();
    let report = director.finish(&env.sock());
    assert_eq!(report.error, Some(ControlError::Cancelled));
    assert_eq!(report.ready, 0);
}

#[test]
fn a_socket_that_cannot_be_bound_is_an_error_at_start() {
    let env = Env::new();
    let mut spec = env.spec(Mode::Make, None);
    spec.sock_path = env.dir.join("missing").join("c.sock");
    assert!(Director::start(spec, no_callback()).is_err());
}

#[test]
fn a_replayed_stop_that_differs_from_the_checkpoint_record_is_answered_with_nothing() {
    for differing in [
        Message {
            stop_seq: 4,
            ..ready()
        },
        Message {
            branch_count: 1001,
            ..ready()
        },
        Message {
            vtime_ns: 6,
            ..ready()
        },
        Message {
            vcpu_id: 3,
            ..ready()
        },
    ] {
        let env = Env::new();
        env.write_log(&[stop_line()]);
        let director = Director::start(env.spec(Mode::Restore, None), no_callback()).unwrap();
        let mut peer = connect(&env);
        send(&mut peer, &differing);
        assert!(answer(&mut peer).is_none(), "no RESUME is sent");
        let report = director.finish(&env.sock());
        assert_eq!(report.resume_sent, 0);
        assert!(
            matches!(report.resume, ResumeResult::Unseen(ref t) if t.contains("differs from the checkpoint record"))
        );
    }
}

#[test]
fn the_progress_is_pending_while_it_runs_then_done_or_failed() {
    let env = Env::new();
    env.write_log(&[stop_line()]);
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    assert_eq!(director.progress(), Progress::Pending);
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    let _ = answer(&mut peer);
    while !director.is_finished() {
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(20));
    assert_eq!(director.progress(), Progress::Done);
    director.finish(&env.sock());
    let env = Env::new();
    let director = Director::start(env.spec(Mode::Make, None), no_callback()).unwrap();
    director.cancel();
    while !director.is_finished() {
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(20));
    assert_eq!(director.progress(), Progress::Failed);
    director.finish(&env.sock());
}

#[test]
fn a_line_with_hostile_text_is_kept_as_printable_ascii_in_the_private_field_only() {
    let env = Env::new();
    let mut raw = format!("[det_counter=1] {}\n", stop_line()).into_bytes();
    raw.extend_from_slice(b"[det_counter=2] deterministic \x1b[31m \xff\xfe\n");
    std::fs::write(env.log(), raw).unwrap();
    let director = Director::start(env.spec(Mode::Restore, None), no_callback()).unwrap();
    let mut peer = connect(&env);
    send(&mut peer, &ready());
    let _ = answer(&mut peer);
    let report = director.finish(&env.sock());
    let line = report.first_line.unwrap();
    assert!(line.bytes().all(|b| (0x20..0x7f).contains(&b)), "{line:?}");
    let ResumeResult::Unseen(text) = report.resume else {
        panic!("not unseen");
    };
    assert!(
        !text.contains("deterministic") && !text.contains('\u{1b}'),
        "{text}"
    );
}
