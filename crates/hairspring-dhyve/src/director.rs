//! The control endpoint. This process listens on a Unix socket whose path it gives the VM program; the VM program connects
//! to it (it does not listen) and sends 72-byte messages. The endpoint answers each stop:
//!
//! * `Make`: at the first stop it runs a callback (the checkpoint), checks the stop is still held, sends SHUTDOWN, waits a
//!   fixed delay and closes.
//! * `Restore`: the first event is the restore's own replayed stop. It answers RESUME and closes only once `run.log`
//!   shows the command was taken (the first log line after the stop that is not a skipped kind is the connect line), at
//!   the latest after a limit, or when the VM has exited. Closing sooner can lose the command.
//!
//! Any STOPPED event is answered with SHUTDOWN and counted: it is how a trial that ran into the branch limit looks.
use crate::runlog::{stop_held_in, RestoreScan, RestoreScanKeys, ScanState};
use crate::safe_open::open_regular;
use crate::wire::{Kind, Message, WireError, SIZE};
use std::io::{BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a blocked read or accept waits before looking at the stop flags.
const LOOK_EVERY: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Make,
    Restore,
}

/// Everything the endpoint needs to know.
#[derive(Debug, Clone)]
pub struct DirectorSpec {
    pub sock_path: PathBuf,
    pub mode: Mode,
    pub run_log: PathBuf,
    /// A restore's keys and restored pointer, for reading its log.
    pub restore: Option<RestoreScanKeys>,
    pub connect_wait: Duration,
    pub resume_limit: Duration,
    pub resume_poll: Duration,
    pub shutdown_delay: Duration,
}

/// Why the connection was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWhy {
    Seen,
    Excluded,
    Other,
    Limit,
    Exited,
    Delay,
    NotHeld,
    Incomplete,
}

/// What came of the RESUME command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeResult {
    NotSent,
    Seen,
    /// Sent, and the log did not show it taken. The text is the reason and, where there is one, the line.
    Unseen(String),
}

/// Why the endpoint ended with no clean close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    NoConnect,
    Lost(String),
    Wire(WireError),
    Cancelled,
}

/// What happened on the control channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub connected: bool,
    pub ready: u32,
    pub stopped: u32,
    pub resume_sent: u32,
    pub shutdown_sent: bool,
    /// The first READY event (in a make run, the stop the checkpoint is taken at).
    pub ready_event: Option<Message>,
    pub resume: ResumeResult,
    /// Whether the stop was still held when the shutdown was due (make runs).
    pub held: Option<bool>,
    pub close: Option<CloseWhy>,
    pub error: Option<ControlError>,
    /// The callback's own reason, when it said not to go on.
    pub callback_failure: Option<String>,
    /// Lines of the skipped kinds that were passed while looking for the connect line.
    pub skipped: u32,
    /// The first line that decided a resume was not taken, as printable ASCII. For the private record only; it is never
    /// put in an error text.
    pub first_line: Option<String>,
}

impl Report {
    fn new() -> Report {
        Report {
            connected: false,
            ready: 0,
            stopped: 0,
            resume_sent: 0,
            shutdown_sent: false,
            ready_event: None,
            resume: ResumeResult::NotSent,
            held: None,
            close: None,
            error: None,
            callback_failure: None,
            skipped: 0,
            first_line: None,
        }
    }
}

/// Run at a make run's first stop. `Err` says not to go on (no shutdown is sent).
pub type OnReady = Box<dyn FnMut(&Message) -> Result<(), String> + Send>;

/// How far the endpoint has got, readable while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Pending,
    /// The task is done as it should be: the shutdown was sent (make) or the resume was taken (restore).
    Done,
    /// The endpoint ended and the task was not done as it should be.
    Failed,
}

const PENDING: u8 = 0;
const DONE: u8 = 1;
const FAILED: u8 = 2;

fn progress_of(report: &Report, mode: Mode) -> u8 {
    let clean = report.error.is_none() && report.stopped == 0;
    let done = match mode {
        Mode::Make => report.shutdown_sent && report.close == Some(CloseWhy::Delay),
        Mode::Restore => report.resume == ResumeResult::Seen,
    };
    if clean && done {
        DONE
    } else {
        FAILED
    }
}

/// A running endpoint.
pub struct Director {
    thread: JoinHandle<Report>,
    cancel: Arc<AtomicBool>,
    vm_exited: Arc<AtomicBool>,
    progress: Arc<AtomicU8>,
}

impl Director {
    /// Binds the socket and starts serving. Fails if the socket cannot be bound.
    pub fn start(spec: DirectorSpec, on_ready: OnReady) -> std::io::Result<Director> {
        if let Err(e) = std::fs::remove_file(&spec.sock_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e);
            }
        }
        let listener = UnixListener::bind(&spec.sock_path)?;
        listener.set_nonblocking(true)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let vm_exited = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU8::new(PENDING));
        let ctx = Context {
            spec,
            cancel: Arc::clone(&cancel),
            vm_exited: Arc::clone(&vm_exited),
        };
        let mark = Arc::clone(&progress);
        let thread = thread::spawn(move || {
            let report = serve(&ctx, &listener, on_ready);
            mark.store(progress_of(&report, ctx.spec.mode), Ordering::SeqCst);
            report
        });
        Ok(Director {
            thread,
            cancel,
            vm_exited,
            progress,
        })
    }

    /// How far the endpoint has got.
    pub fn progress(&self) -> Progress {
        match self.progress.load(Ordering::SeqCst) {
            DONE => Progress::Done,
            FAILED => Progress::Failed,
            _ => Progress::Pending,
        }
    }

    /// Tells the endpoint the VM process has exited.
    pub fn vm_exited(&self) {
        self.vm_exited.store(true, Ordering::SeqCst);
    }

    /// Asks the endpoint to stop at its next look.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for the endpoint to end and returns what happened. The socket file is removed.
    pub fn finish(self, sock_path: &Path) -> Report {
        let report = self.thread.join().unwrap_or_else(|_| {
            let mut r = Report::new();
            r.error = Some(ControlError::Lost(
                "the control thread ended abnormally".to_string(),
            ));
            r
        });
        let _ = std::fs::remove_file(sock_path);
        report
    }
}

struct Context {
    spec: DirectorSpec,
    cancel: Arc<AtomicBool>,
    vm_exited: Arc<AtomicBool>,
}

impl Context {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

fn accept(ctx: &Context, listener: &UnixListener) -> Result<UnixStream, ControlError> {
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((conn, _)) => return Ok(conn),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(ControlError::Lost(e.kind().to_string())),
        }
        if ctx.cancelled() {
            return Err(ControlError::Cancelled);
        }
        if started.elapsed() >= ctx.spec.connect_wait {
            return Err(ControlError::NoConnect);
        }
        thread::sleep(LOOK_EVERY);
    }
}

fn read_message(ctx: &Context, conn: &mut UnixStream) -> Result<Option<Message>, ControlError> {
    fill(ctx, conn, [0u8; SIZE], 0)
}

/// Reads the rest of a message of which `got` bytes are in `buf`. `Ok(None)` is end of stream at a message boundary.
fn fill(
    ctx: &Context,
    conn: &mut UnixStream,
    mut buf: [u8; SIZE],
    mut got: usize,
) -> Result<Option<Message>, ControlError> {
    loop {
        if got == SIZE {
            return Message::decode(&buf).map(Some).map_err(ControlError::Wire);
        }
        match conn.read(&mut buf[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ControlError::Lost("a message ended early".to_string())),
            Ok(n) => got += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if ctx.cancelled() {
                    return Err(ControlError::Cancelled);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ControlError::Lost(e.kind().to_string())),
        }
    }
}

/// Looks for a message for up to one poll interval while the log is being read. Once the VM has closed its end, only waits.
fn poll_event(
    ctx: &Context,
    conn: &mut UnixStream,
    closed: &mut bool,
) -> Result<Option<Message>, ControlError> {
    if *closed {
        thread::sleep(ctx.spec.resume_poll);
        return Ok(None);
    }
    let _ = conn.set_read_timeout(Some(ctx.spec.resume_poll));
    let mut buf = [0u8; SIZE];
    let first = conn.read(&mut buf);
    let _ = conn.set_read_timeout(Some(LOOK_EVERY));
    match first {
        Ok(0) => {
            *closed = true;
            Ok(None)
        }
        Ok(n) => fill(ctx, conn, buf, n),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(ControlError::Lost(e.kind().to_string())),
    }
}

fn send(conn: &mut UnixStream, message: &Message) -> Result<(), ControlError> {
    conn.write_all(&message.encode())
        .map_err(|e| ControlError::Lost(e.kind().to_string()))
}

fn serve(ctx: &Context, listener: &UnixListener, mut on_ready: OnReady) -> Report {
    let mut report = Report::new();
    let mut conn = match accept(ctx, listener) {
        Ok(c) => c,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    report.connected = true;
    if conn.set_nonblocking(false).is_err() || conn.set_read_timeout(Some(LOOK_EVERY)).is_err() {
        report.error = Some(ControlError::Lost(
            "the connection could not be set up".to_string(),
        ));
        return report;
    }
    loop {
        match read_message(ctx, &mut conn) {
            Ok(Some(event)) => {
                if let Err(e) = handle(ctx, &mut conn, &event, &mut on_ready, &mut report) {
                    report.error = Some(e);
                    return report;
                }
                if report.close.is_some() {
                    return report;
                }
            }
            Ok(None) => {
                report.error = Some(ControlError::Lost(
                    "the VM closed the connection".to_string(),
                ));
                return report;
            }
            Err(e) => {
                report.error = Some(e);
                return report;
            }
        }
    }
}

fn close(report: &mut Report, conn: &UnixStream, why: CloseWhy) {
    let _ = conn.shutdown(std::net::Shutdown::Both);
    report.close = Some(why);
}

fn handle(
    ctx: &Context,
    conn: &mut UnixStream,
    event: &Message,
    on_ready: &mut OnReady,
    report: &mut Report,
) -> Result<(), ControlError> {
    match event.kind() {
        Some(Kind::Ready) => {
            report.ready += 1;
            if report.ready_event.is_none() {
                report.ready_event = Some(*event);
            }
            match ctx.spec.mode {
                Mode::Make => make(ctx, conn, event, on_ready, report),
                Mode::Restore => restore(ctx, conn, event, report),
            }
        }
        Some(Kind::Stopped) => {
            report.stopped += 1;
            shutdown_and_close(ctx, conn, event, report, false)
        }
        _ => Ok(()),
    }
}

fn make(
    ctx: &Context,
    conn: &mut UnixStream,
    event: &Message,
    on_ready: &mut OnReady,
    report: &mut Report,
) -> Result<(), ControlError> {
    if let Err(why) = on_ready(event) {
        report.callback_failure = Some(why);
        close(report, conn, CloseWhy::Incomplete);
        return Ok(());
    }
    shutdown_and_close(ctx, conn, event, report, true)
}

fn shutdown_and_close(
    ctx: &Context,
    conn: &mut UnixStream,
    event: &Message,
    report: &mut Report,
    check_held: bool,
) -> Result<(), ControlError> {
    if check_held {
        let held = open_regular(&ctx.spec.run_log)
            .ok()
            .and_then(|f| stop_held_in(BufReader::new(f), event.stop_seq, event.branch_count));
        report.held = Some(held == Some(true));
        if held != Some(true) {
            close(report, conn, CloseWhy::NotHeld);
            return Ok(());
        }
    }
    send(conn, &Message::answer(Kind::Shutdown, event))?;
    report.shutdown_sent = true;
    thread::sleep(ctx.spec.shutdown_delay);
    close(report, conn, CloseWhy::Delay);
    Ok(())
}

fn read_new(path: &Path, offset: u64) -> (Vec<u8>, u64) {
    let Ok(mut file) = open_regular(path) else {
        return (Vec::new(), offset);
    };
    let mut data = Vec::new();
    let read = std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(offset))
        .and_then(|_| file.read_to_end(&mut data));
    match read {
        Ok(n) => (data, offset + n as u64),
        Err(_) => (Vec::new(), offset),
    }
}

fn restore(
    ctx: &Context,
    conn: &mut UnixStream,
    event: &Message,
    report: &mut Report,
) -> Result<(), ControlError> {
    let Some(keys) = ctx.spec.restore.clone() else {
        report.resume = ResumeResult::Unseen("no restore keys".to_string());
        close(report, conn, CloseWhy::Other);
        return Ok(());
    };
    // The replayed stop must be the one the checkpoint recorded; answering another would be answering the wrong stop.
    if event.stop_seq != keys.stop_seq
        || event.branch_count != keys.branch_count
        || event.vtime_ns != keys.vtime_ns
    {
        report.resume = ResumeResult::Unseen(
            "the replayed stop differs from the checkpoint record".to_string(),
        );
        close(report, conn, CloseWhy::Other);
        return Ok(());
    }
    send(conn, &Message::answer(Kind::Resume, event))?;
    report.resume_sent += 1;
    let mut scan = RestoreScan::new(keys);
    let started = Instant::now();
    let mut offset = 0u64;
    let mut closed = false;
    loop {
        let elapsed = started.elapsed();
        let (data, next) = read_new(&ctx.spec.run_log, offset);
        offset = next;
        let state = scan.feed(&data).clone();
        report.skipped = scan.skipped();
        let exited = ctx.vm_exited.load(Ordering::SeqCst);
        match state {
            ScanState::Seen => {
                let on_time = elapsed <= ctx.spec.resume_limit;
                report.resume = if on_time {
                    ResumeResult::Seen
                } else {
                    ResumeResult::Unseen("limit".to_string())
                };
                close(
                    report,
                    conn,
                    if on_time {
                        CloseWhy::Seen
                    } else {
                        CloseWhy::Limit
                    },
                );
                return Ok(());
            }
            ScanState::Excluded(line) => {
                report.first_line = Some(line);
                report.resume = ResumeResult::Unseen("the VM refused the command".to_string());
                close(report, conn, CloseWhy::Excluded);
                return Ok(());
            }
            ScanState::Other(line) => {
                report.first_line = Some(line);
                report.resume = ResumeResult::Unseen("another line came first".to_string());
                close(report, conn, CloseWhy::Other);
                return Ok(());
            }
            ScanState::Pending => {}
        }
        if exited || elapsed >= ctx.spec.resume_limit || ctx.cancelled() {
            let why = if exited { "the VM exited" } else { "limit" };
            report.resume = ResumeResult::Unseen(why.to_string());
            close(
                report,
                conn,
                if exited {
                    CloseWhy::Exited
                } else {
                    CloseWhy::Limit
                },
            );
            return Ok(());
        }
        if let Some(event) = poll_event(ctx, conn, &mut closed)? {
            if event.kind() == Some(Kind::Stopped) {
                report.stopped += 1;
                report.resume = ResumeResult::Unseen("the VM stopped again".to_string());
                return shutdown_and_close(ctx, conn, &event, report, false);
            }
        }
    }
}

#[cfg(test)]
#[path = "director_tests.rs"]
mod tests;
