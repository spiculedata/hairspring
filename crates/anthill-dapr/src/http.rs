//! A deliberately tiny HTTP/1.1 server and client over `std::net`: one request per connection,
//! bounded sizes, no dependencies. Simple enough to behave deterministically under Hermit.
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const ACCEPT_POLL: Duration = Duration::from_millis(5);
const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
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
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
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
}

fn read_request(stream: &mut TcpStream) -> Result<Request, ReadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(i) = find_header_end(&buf) {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(ReadError::TooLarge);
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(ReadError::Bad),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.splitn(2, "\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split(' ').collect();
    if parts.len() != 3 || !parts[2].starts_with("HTTP/") || parts[0].is_empty() {
        return Err(ReadError::Bad);
    }
    let headers = parse_headers(lines.next().unwrap_or(""));
    let length = match header(&headers, "content-length") {
        None => 0,
        Some(v) => v.parse::<usize>().map_err(|_| ReadError::Bad)?,
    };
    if length > MAX_BODY_BYTES {
        return Err(ReadError::TooLarge);
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(ReadError::Bad),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(length);
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

/// Serves `handler` on `listener` until `stop` is set. One thread per connection.
pub fn serve(
    listener: TcpListener,
    handler: Arc<dyn Fn(Request) -> Response + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    let _ = listener.set_nonblocking(true);
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let handler = handler.clone();
                    std::thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(SERVER_READ_TIMEOUT));
                        match read_request(&mut stream) {
                            Ok(req) => write_response(&mut stream, &handler(req)),
                            Err(ReadError::TooLarge) => write_response(
                                &mut stream,
                                &Response::text(413, "request too large"),
                            ),
                            Err(ReadError::Bad) => {
                                write_response(&mut stream, &Response::text(400, "bad request"))
                            }
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL)
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

/// A blocking request with a connect and read timeout.
pub fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<Response, String> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
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
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > MAX_RESPONSE_BYTES {
                    return Err("response too large".to_string());
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(format!("timed out waiting for {addr}"))
            }
            Err(e) => return Err(format!("read from {addr} failed: {e}")),
        }
    }
    let end = find_header_end(&raw).ok_or("malformed response")?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.splitn(2, "\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("malformed status line")?;
    let headers = parse_headers(lines.next().unwrap_or(""));
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
