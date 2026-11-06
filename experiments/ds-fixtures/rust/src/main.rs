//! dsrs: the Rust server of the loopback experiment ring. Same line protocol as the C server dskv; Rust std only, blocking sockets,
//! one thread per connection, ring replication over one persistent link to the successor.
//!
//!     dsrs --node=n1 --port=P --peer-port=Q [--tmp=DIR] [--mode=...]
//!
//! `--tmp` and `--mode` are accepted so that the launcher can pass the same arguments to every server; they are
//! ignored. accept and read have no bound by design (blocking std sockets); the launcher bounds the process by QUIT
//! and, failing that, by SIGKILL. Experiment fixture, not product code. Nothing here has been run under Hermit.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::mem::ManuallyDrop;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;
use std::process::exit;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use dsrs::Server;

const CONNECT_TRIES: u32 = 200;
const RETRY_SLEEP: Duration = Duration::from_millis(5);
/// Orphan guard: exit after this many one-second ticks with no request.
const IDLE_TICKS: u32 = 120;
static ACTIVITY: AtomicU64 = AtomicU64::new(0);

fn fatal(msg: &str) -> ! {
    eprintln!("error: {msg}");
    exit(2)
}

/// One line to stdout with a single write.
fn emit(line: &str) {
    // SAFETY: descriptor 1 is open for the life of the process; ManuallyDrop keeps it open.
    let mut out = ManuallyDrop::new(unsafe { File::from_raw_fd(1) });
    let _ = out.write_all(line.as_bytes());
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let prefix = format!("--{name}=");
    args.iter().find_map(|a| a.strip_prefix(prefix.as_str()))
}

fn port_of(args: &[String], name: &str) -> u16 {
    opt(args, name)
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p != 0)
        .unwrap_or_else(|| {
            fatal(&format!(
                "usage: dsrs --node=n1 --port=P --peer-port=Q (bad or missing --{name})"
            ))
        })
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn connect_successor(server: &Mutex<Server>, port: u16) -> TcpStream {
    for _ in 0..CONNECT_TRIES {
        match TcpStream::connect(loopback(port)) {
            Ok(s) => return s,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                lock(server).refused += 1;
                thread::sleep(RETRY_SLEEP);
            }
            Err(_) => break,
        }
    }
    fatal("cannot reach the successor")
}

fn serve(stream: TcpStream, server: Arc<Mutex<Server>>, succ: Arc<Mutex<TcpStream>>) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        ACTIVITY.fetch_add(1, Ordering::Relaxed);
        let act = lock(&server).handle(line.trim_end(), true);
        if let Some(r) = act.replicate {
            let _ = lock(&succ).write_all(r.as_bytes());
        }
        if let Some(r) = act.reply {
            let _ = writer.write_all(r.as_bytes());
        }
        if act.quit {
            emit(&lock(&server).result_line());
            exit(0);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let node = opt(&args, "node")
        .filter(|n| dsrs::valid_name(n))
        .unwrap_or_else(|| fatal("usage: dsrs --node=n1 --port=P --peer-port=Q (a node name is 1 to 16 letters and digits)"));
    let port = port_of(&args, "port");
    let peer_port = port_of(&args, "peer-port");

    let server = Arc::new(Mutex::new(Server::new(node)));
    let listener = TcpListener::bind(loopback(port)).unwrap_or_else(|_| fatal("cannot listen"));
    let mut link = connect_successor(&server, peer_port);
    if link
        .write_all(format!("HELLO {node}\n").as_bytes())
        .is_err()
    {
        fatal("cannot reach the successor");
    }
    // Anything the successor sends back (heartbeats from other implementations) is read and dropped.
    if let Ok(mut back) = link.try_clone() {
        thread::spawn(move || {
            let mut sink = [0u8; 256];
            while std::io::Read::read(&mut back, &mut sink)
                .map(|n| n > 0)
                .unwrap_or(false)
            {}
        });
    }
    let succ = Arc::new(Mutex::new(link));

    let watched = Arc::clone(&server);
    thread::spawn(move || {
        let (mut last, mut idle) = (ACTIVITY.load(Ordering::Relaxed), 0);
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = ACTIVITY.load(Ordering::Relaxed);
            idle = if now == last { idle + 1 } else { 0 };
            last = now;
            if idle >= IDLE_TICKS {
                emit(&lock(&watched).result_line());
                exit(4);
            }
        }
    });

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (server, succ) = (Arc::clone(&server), Arc::clone(&succ));
        thread::spawn(move || serve(stream, server, succ));
    }
}
