//! A deliberately tiny HTTP/1.1 server and client over `std::net`: one request per connection,
//! bounded sizes, deadlines and connection caps, no dependencies. Simple enough to behave
//! deterministically under Hermit.
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const ACCEPT_POLL: Duration = Duration::from_millis(5);
const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const DEFAULT_MAX_CONNECTIONS: usize = 256;

/// Limits that keep one slow or greedy client from starving the others.
#[derive(Debug, Clone, Copy)]
pub struct ServeLimits {
    /// The whole request (headers and body) must arrive within this time, however slowly it trickles.
    pub request_deadline: Duration,
    /// Concurrent connections served; extra ones get `503`.
    pub max_connections: usize,
}

impl Default for ServeLimits {
    fn default() -> Self {
        ServeLimits {
            request_deadline: DEFAULT_REQUEST_DEADLINE,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, v: &Value) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: v.to_string().into_bytes(),
        }
    }
    pub fn text(status: u16, s: &str) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: s.as_bytes().to_vec(),
        }
    }
    pub fn empty(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    pub fn with_header(mut self, k: &str, v: &str) -> Response {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn write_response(stream: &mut TcpStream, r: &Response) {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.status,
        reason(r.status),
        r.body.len()
    );
    for (k, v) in &r.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(&r.body);
    let _ = stream.flush();
}

enum ReadError {
    Bad,
    TooLarge,
    /// The client was too slow; the connection is dropped without a reply.
    Slow,
}

/// One read bounded by what is left of the request deadline.
fn read_within(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<usize, ReadError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(ReadError::Slow);
    }
    stream
        .set_read_timeout(Some(left))
        .map_err(|_| ReadError::Bad)?;
    match stream.read(buf) {
        Ok(0) => Err(ReadError::Bad),
        Ok(n) => Ok(n),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Err(ReadError::Slow)
        }
        Err(_) => Err(ReadError::Bad),
    }
}

fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<Request, ReadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(i) = find_header_end(&buf) {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(ReadError::TooLarge);
        }
        let n = read_within(stream, &mut chunk, deadline)?;
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.splitn(2, "\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split(' ').collect();
    if parts.len() != 3 || !parts[2].starts_with("HTTP/") || parts[0].is_empty() {
        return Err(ReadError::Bad);
    }
    let headers = parse_headers(lines.next().unwrap_or(""));
    let mut body = buf[header_end + 4..].to_vec();
    if header(&headers, "transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        while !body.ends_with(b"0\r\n\r\n") {
            if body.len() > MAX_BODY_BYTES {
                return Err(ReadError::TooLarge);
            }
            let n = read_within(stream, &mut chunk, deadline)?;
            body.extend_from_slice(&chunk[..n]);
        }
        body = decode_chunked(&body);
    } else {
        let length = match header(&headers, "content-length") {
            None => 0,
            Some(v) => v.parse::<usize>().map_err(|_| ReadError::Bad)?,
        };
        if length > MAX_BODY_BYTES {
            return Err(ReadError::TooLarge);
        }
        while body.len() < length {
            let n = read_within(stream, &mut chunk, deadline)?;
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(length);
    }
    let (path, query) = parts[1]
        .split_once('?')
        .map(|(p, q)| (p.to_string(), q.to_string()))
        .unwrap_or((parts[1].to_string(), String::new()));
    Ok(Request {
        method: parts[0].to_string(),
        path,
        query,
        headers,
        body,
    })
}

/// Decrements the live-connection count when a connection ends, however it ends.
struct ConnectionSlot(Arc<AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serves `handler` on `listener` until `stop` is set, with default limits.
pub fn serve(
    listener: TcpListener,
    handler: Arc<dyn Fn(Request) -> Response + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    serve_with(listener, handler, stop, ServeLimits::default());
}

/// Serves `handler` with explicit limits. One thread per connection, at most `max_connections` at once.
pub fn serve_with(
    listener: TcpListener,
    handler: Arc<dyn Fn(Request) -> Response + Send + Sync>,
    stop: Arc<AtomicBool>,
    limits: ServeLimits,
) {
    let _ = listener.set_nonblocking(true);
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if active.fetch_add(1, Ordering::SeqCst) >= limits.max_connections {
                        active.fetch_sub(1, Ordering::SeqCst);
                        write_response(&mut stream, &Response::text(503, "too many connections"));
                        continue;
                    }
                    let slot = ConnectionSlot(active.clone());
                    let handler = handler.clone();
                    std::thread::spawn(move || {
                        let _slot = slot;
                        let _ = stream.set_nonblocking(false);
                        let deadline = Instant::now() + limits.request_deadline;
                        match read_request(&mut stream, deadline) {
                            Ok(req) => write_response(&mut stream, &handler(req)),
                            Err(ReadError::TooLarge) => write_response(
                                &mut stream,
                                &Response::text(413, "request too large"),
                            ),
                            Err(ReadError::Bad) => {
                                write_response(&mut stream, &Response::text(400, "bad request"))
                            }
                            Err(ReadError::Slow) => {}
                        }
                    });
                }
                Err(_) => std::thread::sleep(ACCEPT_POLL),
            }
        }
    });
}

fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&rest[..pos])
                .split(';')
                .next()
                .unwrap_or("")
                .trim(),
            16,
        )
        .unwrap_or(0);
        rest = &rest[pos + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
    out
}

/// A blocking request. `timeout` bounds the whole exchange (connect, send and receive), not each read.
pub fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<Response, String> {
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| format!("write to {addr} failed: {e}"))?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(end) = find_header_end(&raw) {
            let parsed = parse_head(&raw[..end]);
            if let Some(len) = parsed
                .as_ref()
                .and_then(|(_, h)| header(h, "content-length"))
                .and_then(|v| v.parse::<usize>().ok())
            {
                if raw.len() >= end + 4 + len {
                    break; // the whole body has arrived; no need to wait for the server to close
                }
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("timed out waiting for {addr}"));
        }
        stream
            .set_read_timeout(Some(left))
            .map_err(|e| e.to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > MAX_RESPONSE_BYTES {
                    return Err("response too large".to_string());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(format!("timed out waiting for {addr}"))
            }
            Err(e) => return Err(format!("read from {addr} failed: {e}")),
        }
    }
    let end = find_header_end(&raw).ok_or("malformed response")?;
    let (status, headers) = parse_head(&raw[..end]).ok_or("malformed status line")?;
    let body = &raw[end + 4..];
    let body = if header(&headers, "transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        decode_chunked(body)
    } else {
        body.to_vec()
    };
    Ok(Response {
        status,
        headers,
        body,
    })
}

fn parse_head(head: &[u8]) -> Option<(u16, Vec<(String, String)>)> {
    let head = String::from_utf8_lossy(head).into_owned();
    let mut lines = head.splitn(2, "\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse::<u16>().ok()?;
    Some((status, parse_headers(lines.next().unwrap_or(""))))
}
