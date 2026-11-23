//! The fake `bhyve`: connects to the control socket it is given and plays a VM's part, as scripted by the `modes` file.
//!
//! Words a start can be given: `no_connect`, `drop_before_ready`, `no_ready`, `not_held`, `shutdown_exit=N` (make runs);
//! `stale_stop`, `other_line`, `no_skipped_lines`, `rip_mismatch`, `drop_after_resume`, `limit_open`, `limit_log`,
//! `log_overrun`, `hang`, `no_uart`, `empty_uart`, `no_exit`, `exit=N`, `signal=N` (restore runs).
use super::scenario::{environment_ok, Fake};
use hairspring_dhyve::ckptmeta::restored_rip;
use hairspring_dhyve::runlog::{connect_text, stop_text};
use hairspring_dhyve::vmname::VmName;
use hairspring_dhyve::wire::{Kind, Message, RESUME_BRANCH_TARGET, SIZE};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

const WAIT_FOR_PEER: Duration = Duration::from_secs(20);
const IDLE_STEP: Duration = Duration::from_millis(20);
const IGNORE_DESTROY_FOR: Duration = Duration::from_secs(3);
/// What a hostile guest makes the VM log: option-like words, an escape sequence and bytes that are not text.
const HOSTILE: &[u8] = b"deterministic.seed=1 \x1b[31mred\x1b[0m \xff\xfe -o x=y";
/// What the fake's guest writes to `run.log` in one go when told to overrun.
const OVERRUN_BYTES: usize = 1 << 20;
/// A stop the make run reports.
const MAKE_STOP: Message = Message {
    kind: 1,
    stop_seq: 1,
    branch_count: 395_000,
    vtime_ns: 777,
    probe_id: 0,
    arg0: 0,
    arg1: 0,
    arg2: 0,
    vcpu_id: 0,
};

extern "C" {
    fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
}

struct Parsed {
    options: BTreeMap<String, String>,
    fwcfg: BTreeMap<String, String>,
    restore: Option<String>,
    name: String,
}

fn parse(args: &[String]) -> Option<Parsed> {
    let mut p = Parsed {
        options: BTreeMap::new(),
        fwcfg: BTreeMap::new(),
        restore: None,
        name: args.last()?.clone(),
    };
    let mut i = 0;
    while i + 1 < args.len() {
        let value = &args[i + 1];
        match args[i].as_str() {
            "-o" => {
                let (k, v) = value.split_once('=')?;
                p.options.insert(k.to_string(), v.to_string());
            }
            "-f" => {
                let rest = value.strip_prefix("name=opt/hs/")?;
                let (field, file) = rest.split_once(",file=")?;
                p.fwcfg.insert(field.to_string(), file.to_string());
            }
            "-r" => p.restore = Some(value.clone()),
            _ => {}
        }
        i += 2;
    }
    Some(p)
}

struct Vm<'a> {
    fake: &'a Fake,
    words: Vec<String>,
    p: Parsed,
    counter: u32,
}

fn word_value(words: &[String], key: &str) -> Option<i32> {
    words
        .iter()
        .find_map(|w| w.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.parse().ok())
}

impl Vm<'_> {
    fn has(&self, word: &str) -> bool {
        self.words.iter().any(|w| w == word)
    }

    fn det(&mut self, text: &str) {
        self.counter += 1;
        eprintln!("[det_counter={}] {text}", self.counter);
    }

    fn option(&self, key: &str) -> &str {
        self.p.options.get(key).map_or("", String::as_str)
    }

    fn uart(&self) -> PathBuf {
        PathBuf::from(self.option("dhyve.director.uart_log"))
    }

    fn uart_line(&self, text: &str) {
        if self.has("no_uart") || self.has("uart_fifo") {
            return;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.uart())
        {
            let _ = writeln!(f, "{text}");
        }
    }

    fn touch_uart(&self) {
        let uart = self.uart();
        if self.has("uart_link") {
            let real = uart.with_file_name("real-uart");
            let _ = std::fs::write(&real, b"");
            let _ = std::os::unix::fs::symlink(&real, &uart);
            return;
        }
        if self.has("uart_fifo") {
            if let Ok(c) = std::ffi::CString::new(uart.display().to_string()) {
                // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
                unsafe {
                    mkfifo(c.as_ptr(), 0o600);
                }
            }
            return;
        }
        if !self.has("no_uart") {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.uart());
        }
    }

    /// Writes a det line holding raw bytes, as the VM writes the guest's serial text into its log.
    fn det_raw(&mut self, text: &[u8]) {
        self.counter += 1;
        let mut line = format!("[det_counter={}] ", self.counter).into_bytes();
        line.extend_from_slice(text);
        line.push(b'\n');
        let _ = std::io::stderr().write_all(&line);
    }

    fn idle_until_destroyed(&self) -> i32 {
        if self.has("ignore_destroy") {
            // A VM that does not go when it is destroyed. It ends by itself after a short while, so a test leaves nothing.
            sleep(IGNORE_DESTROY_FOR);
            return 0;
        }
        while self.fake.exists(&self.p.name) {
            sleep(IDLE_STEP);
        }
        0
    }

    fn finish(&self, code: i32) -> i32 {
        self.fake.unregister(&self.p.name);
        code
    }
}

fn connect(path: &str) -> Option<UnixStream> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        if let Ok(s) = UnixStream::connect(path) {
            let _ = s.set_read_timeout(Some(WAIT_FOR_PEER));
            return Some(s);
        }
        sleep(Duration::from_millis(10));
    }
    None
}

fn recv(conn: &mut UnixStream) -> Option<Message> {
    let mut buf = [0u8; SIZE];
    conn.read_exact(&mut buf).ok()?;
    Message::decode(&buf).ok()
}

fn send(conn: &mut UnixStream, m: &Message) {
    let _ = conn.write_all(&m.encode());
}

/// Runs one start of the fake `bhyve`; returns its exit status.
pub fn run(fake: &Fake, args: &[String]) -> i32 {
    fake.log_call(&format!("bhyve {}", args.last().map_or("", String::as_str)));
    fake.append("args.log", &args.join(" "));
    if let Err(why) = environment_ok(fake) {
        eprintln!("{why}");
        return 70;
    }
    let Some(p) = parse(args) else {
        eprintln!("bad arguments");
        return 64;
    };
    if VmName::parse(&p.name).is_none() || p.fwcfg.len() != 6 {
        eprintln!("bad name or input files");
        return 64;
    }
    let words = fake.mode_words();
    let existed = fake.register(&p.name);
    let mut vm = Vm {
        fake,
        words,
        p,
        counter: 0,
    };
    if existed {
        fake.log_call(&format!("reused {}", vm.p.name));
        eprintln!("the name exists");
        return 71;
    }
    if vm.has("no_connect") {
        vm.touch_uart();
        let code = vm.idle_until_destroyed();
        return vm.finish(code);
    }
    let Some(mut conn) = connect(vm.option("dhyve.director.control")) else {
        eprintln!("no control socket");
        return vm.finish(69);
    };
    let code = if vm.p.restore.is_some() {
        restore(&mut vm, &mut conn)
    } else {
        make(&mut vm, &mut conn)
    };
    vm.finish(code)
}

fn make(vm: &mut Vm, conn: &mut UnixStream) -> i32 {
    vm.touch_uart();
    vm.uart_line("boot line");
    vm.uart_line("HS_READY");
    if vm.has("drop_before_ready") {
        return 0;
    }
    vm.det("VTIME_INIT");
    vm.det(&stop_text(MAKE_STOP.stop_seq, MAKE_STOP.branch_count));
    if vm.has("not_held") {
        vm.det("VMEXIT reason=HLT");
    }
    if vm.has("no_ready") {
        return vm.idle_until_destroyed();
    }
    send(conn, &MAKE_STOP);
    match recv(conn) {
        Some(m)
            if m.kind() == Some(Kind::Shutdown)
                && m.stop_seq == MAKE_STOP.stop_seq
                && m.vcpu_id == MAKE_STOP.vcpu_id =>
        {
            word_value(&vm.words, "shutdown_exit").unwrap_or(0)
        }
        Some(_) => 65,
        None => 4,
    }
}

fn keys_of(vm: &Vm) -> Option<Message> {
    let get = |k: &str| {
        vm.option(&format!("dhyve.director.restore.{k}"))
            .parse::<u64>()
            .ok()
    };
    Some(Message {
        kind: Kind::Ready.code(),
        stop_seq: get("stop_seq")?,
        branch_count: get("branch_count")?,
        vtime_ns: get("vtime_ns")?,
        probe_id: get("probe_id")?,
        arg0: get("probe_arg0")?,
        vcpu_id: u32::try_from(get("vcpu_id")?).ok()?,
        ..Message::default()
    })
}

fn checkpoint_rip(path: &str) -> Option<u64> {
    let meta = std::fs::read_to_string(format!("{path}.meta")).ok()?;
    let kern = std::fs::read(format!("{path}.kern")).ok()?;
    restored_rip(&meta, &kern).ok()
}

fn restore(vm: &mut Vm, conn: &mut UnixStream) -> i32 {
    vm.touch_uart();
    let Some(stop) = keys_of(vm) else {
        eprintln!("restore keys missing");
        return 66;
    };
    let ckpt = vm.p.restore.clone().unwrap_or_default();
    let Some(rip) = checkpoint_rip(&ckpt) else {
        eprintln!("checkpoint unreadable");
        return 67;
    };
    vm.det(&stop_text(stop.stop_seq, stop.branch_count));
    if !vm.has("no_skipped_lines") {
        let shown = if vm.has("rip_mismatch") {
            rip.wrapping_add(1)
        } else {
            rip
        };
        vm.det(&format!("VMEXIT reason=DEBUG(21) rip=0x{shown:x}"));
        vm.det(&format!(
            "VTIME 0 -> {} (+{} branches)",
            stop.vtime_ns, stop.branch_count
        ));
    }
    if vm.has("stopped_first") {
        send(
            conn,
            &Message {
                kind: Kind::Stopped.code(),
                stop_seq: 99,
                ..Message::default()
            },
        );
        return recv(conn).map_or(4, |_| 0);
    }
    if vm.has("ready_differs") {
        send(
            conn,
            &Message {
                stop_seq: stop.stop_seq + 5,
                ..stop
            },
        );
        return match recv(conn) {
            Some(_) => 0,
            None => vm.idle_until_destroyed(),
        };
    }
    send(conn, &stop);
    let expected = stop.stop_seq + u64::from(vm.has("stale_stop"));
    let taken = recv(conn).is_some_and(|m| {
        m.kind() == Some(Kind::Resume)
            && m.stop_seq == expected
            && m.vcpu_id == stop.vcpu_id
            && m.branch_count == RESUME_BRANCH_TARGET
    });
    if !taken {
        if vm.has("hostile_line") {
            let mut line = b"director stale resume ".to_vec();
            line.extend_from_slice(HOSTILE);
            vm.det_raw(&line);
        } else {
            vm.det(&format!("director stale resume stop_seq={expected}"));
        }
        return vm.idle_until_destroyed();
    }
    if vm.has("drop_after_resume") {
        let _ = conn.shutdown(std::net::Shutdown::Both);
        return vm.idle_until_destroyed();
    }
    if vm.has("limit_open") {
        send(
            conn,
            &Message {
                kind: Kind::Stopped.code(),
                stop_seq: 99,
                ..Message::default()
            },
        );
        return recv(conn).map_or(4, |_| 0);
    }
    if vm.has("hostile_other") {
        vm.det_raw(HOSTILE);
    } else if vm.has("other_line") {
        vm.det("VMEXIT reason=HLT");
    } else {
        let sock = vm.option("dhyve.director.control").to_string();
        vm.det(&connect_text(&sock));
    }
    guest(vm)
}

fn seed_read(vm: &Vm) -> u64 {
    vm.p.fwcfg
        .get("seed")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

fn guest(vm: &mut Vm) -> i32 {
    let seed = seed_read(vm);
    if !vm.has("empty_uart") {
        vm.uart_line("guest started");
    }
    vm.fake
        .append("trials.log", &format!("seed={seed} name={}", vm.p.name));
    if vm.has("log_overrun") {
        let chunk = "x".repeat(1024);
        for _ in 0..OVERRUN_BYTES / chunk.len() {
            vm.det(&chunk);
        }
        return vm.idle_until_destroyed();
    }
    if vm.has("limit_log") {
        sleep(Duration::from_millis(100));
        vm.det("director stop kind=2 stop_seq=99 branch_count=4000000000 probe_id=0");
        return vm.idle_until_destroyed();
    }
    if vm.has("hang") {
        return vm.idle_until_destroyed();
    }
    if vm.has("stop_then_exit") {
        // A stop of a kind that is not the fence, then the VM ends with no exit report.
        sleep(Duration::from_millis(50));
        vm.det("director stop kind=1 stop_seq=5 branch_count=7 probe_id=0");
        return 0;
    }
    for line in vm.fake.events(seed) {
        vm.uart_line(&line);
    }
    if !vm.has("no_exit") {
        let code = word_value(&vm.words, "exit").unwrap_or(0);
        let signal = word_value(&vm.words, "signal").unwrap_or(0);
        vm.uart_line(&format!("HS_EXIT {code} {signal}"));
    }
    0
}
