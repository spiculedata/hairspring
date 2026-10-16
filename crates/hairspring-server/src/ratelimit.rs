use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 10_000;

/// Fixed-window request limiter keyed by caller. `limit == 0` disables it.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(limit: u32) -> RateLimiter {
        RateLimiter::with_window(limit, WINDOW)
    }

    /// A limiter counting over `window` instead of the default minute.
    pub fn with_window(limit: u32, window: Duration) -> RateLimiter {
        RateLimiter {
            limit,
            window,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if the request is allowed.
    pub fn check(&self, key: &str) -> bool {
        if self.limit == 0 {
            return true;
        }
        let now = Instant::now();
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS {
            map.retain(|_, (start, _)| now.duration_since(*start) < self.window);
            // Still full of live windows: fail closed for new callers rather than grow without bound.
            if map.len() >= MAX_KEYS && !map.contains_key(key) {
                return false;
            }
        }
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.limit
    }
}
