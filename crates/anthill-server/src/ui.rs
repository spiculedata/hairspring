use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::Response;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

const FALLBACK_HTML: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Anthill</title>\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head><body style=\"font-family:system-ui;max-width:42rem;margin:3rem auto;padding:0 1rem\">\
<h1>Anthill</h1><p>The server is running, but the web UI has not been built into this binary.</p>\
<p>Build it with <code>make ui</code> and rebuild, or use the REST API under <code>/api</code> and the MCP endpoint at <code>/mcp</code>.</p></body></html>";

/// Content types for the file kinds a built single-page app contains.
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn clean(path: &str) -> Option<String> {
    let p = path.trim_start_matches('/');
    if p.contains("..") || p.contains('\\') || p.contains('\0') {
        return None;
    }
    Some(p.to_string())
}

fn respond(body: Vec<u8>, mime: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'",
        )
        .header(header::X_FRAME_OPTIONS, "DENY")
        .body(Body::from(body))
        .unwrap_or_default()
}

/// Serves embedded assets; unknown paths fall back to the single-page app shell.
pub async fn handler(uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") || path == "/api" || path.starts_with("/mcp") {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"error\":\"not found\"}"))
            .unwrap_or_default();
    }
    if let Some(rel) = clean(path).filter(|p| !p.is_empty()) {
        if let Some(file) = Assets::get(&rel) {
            return respond(file.data.into_owned(), mime_for(&rel));
        }
    }
    match Assets::get("index.html") {
        Some(index) => respond(index.data.into_owned(), "text/html; charset=utf-8"),
        None => respond(
            FALLBACK_HTML.as_bytes().to_vec(),
            "text/html; charset=utf-8",
        ),
    }
}
