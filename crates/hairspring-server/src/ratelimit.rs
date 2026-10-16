use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 10_000;

/// Fixed-window request limiter keyed by caller. `limit == 0` disables it.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    max_keys: usize,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(limit: u32) -> RateLimiter {
        RateLimiter::with_window(limit, WINDOW)
    }

    /// A limiter counting over `window` instead of the default minute.
    pub fn with_window(limit: u32, window: Duration) -> RateLimiter {
        RateLimiter::with_capacity(limit, window, MAX_KEYS)
    }

    /// A limiter holding at most `max_keys` caller windows.
    pub fn with_capacity(limit: u32, window: Duration, max_keys: usize) -> RateLimiter {
        RateLimiter {
            limit,
            window,
            max_keys,
            windows: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.windows.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Returns true if the request is allowed.
    pub fn check(&self, key: &str) -> bool {
        if self.limit == 0 {
            return true;
        }
        let now = Instant::now();
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.max_keys {
            map.retain(|_, (start, _)| now.duration_since(*start) < self.window);
            // Still full of live windows. Anonymous keys are the only ones a stranger can mint in bulk, so they
            // are what gets evicted (oldest first); token keys are bounded by the configured tokens and stay.
            if map.len() >= self.max_keys && !map.contains_key(key) {
                evict_oldest_anonymous(&mut map, self.max_keys);
                if map.len() >= self.max_keys {
                    return false;
                }
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

const ANONYMOUS_PREFIX: &str = "ip:";
/// IPv6 callers are grouped by this many leading bits: a single subscriber commonly holds a whole /64.
const IPV6_PREFIX_BITS: u32 = 64;

/// The limiter key for a caller with no valid token: the address for IPv4, the /64 for IPv6. An IPv4-mapped
/// IPv6 address counts as its IPv4 address.
pub fn anonymous_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => format!("{ANONYMOUS_PREFIX}{v4}"),
        IpAddr::V6(v6) => {
            let mask = u128::MAX << (128 - IPV6_PREFIX_BITS);
            let net = Ipv6Addr::from(u128::from(v6) & mask);
            format!("{ANONYMOUS_PREFIX}{net}/{IPV6_PREFIX_BITS}")
        }
    }
}

/// Frees room by dropping the oldest tenth (at least one) of the anonymous windows.
fn evict_oldest_anonymous(map: &mut HashMap<String, (Instant, u32)>, max_keys: usize) {
    let mut anonymous: Vec<(Instant, String)> = map
        .iter()
        .filter(|(k, _)| k.starts_with(ANONYMOUS_PREFIX))
        .map(|(k, (start, _))| (*start, k.clone()))
        .collect();
    anonymous.sort();
    let batch = (max_keys / 10).max(1);
    for (_, key) in anonymous.into_iter().take(batch) {
        map.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const LONG: Duration = Duration::from_secs(3600);

    fn v6(last: u16, host: u16) -> IpAddr {
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0, 0, last, host))
    }

    #[test]
    fn ipv6_addresses_in_one_64_share_a_key() {
        assert_eq!(anonymous_key(v6(1, 1)), anonymous_key(v6(9, 77)));
    }

    #[test]
    fn ipv6_addresses_in_different_64s_have_different_keys() {
        let other = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 3, 0, 0, 1, 1));
        assert_ne!(anonymous_key(v6(1, 1)), anonymous_key(other));
    }

    #[test]
    fn ipv4_callers_are_keyed_by_address() {
        let a = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let b = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
        assert_ne!(anonymous_key(a), anonymous_key(b));
        assert_eq!(anonymous_key(a), anonymous_key(a));
    }

    #[test]
    fn an_ipv4_mapped_ipv6_address_is_keyed_as_its_ipv4_address() {
        let v4 = Ipv4Addr::new(192, 0, 2, 7);
        assert_eq!(
            anonymous_key(IpAddr::V6(v4.to_ipv6_mapped())),
            anonymous_key(IpAddr::V4(v4))
        );
    }

    #[test]
    fn a_flood_of_anonymous_addresses_never_blocks_an_authenticated_token() {
        let limiter = RateLimiter::with_capacity(5, LONG, 100);
        assert!(limiter.check("t:abc"));
        for i in 0..1_000u32 {
            let ip = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
            limiter.check(&anonymous_key(ip));
        }
        // The token keeps its own count, and a token first seen after the flood is still admitted.
        assert!(limiter.check("t:abc"));
        assert!(limiter.check("t:new"));
    }

    #[test]
    fn a_new_anonymous_caller_is_still_admitted_when_the_table_is_full() {
        let limiter = RateLimiter::with_capacity(5, LONG, 100);
        for i in 0..1_000u32 {
            limiter.check(&anonymous_key(IpAddr::V4(Ipv4Addr::from(i))));
        }
        let late = anonymous_key(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)));
        assert!(limiter.check(&late));
    }

    #[test]
    fn the_table_does_not_grow_past_its_capacity() {
        let limiter = RateLimiter::with_capacity(5, LONG, 100);
        for i in 0..1_000u32 {
            limiter.check(&anonymous_key(IpAddr::V4(Ipv4Addr::from(i))));
        }
        assert!(limiter.len() <= 100);
    }

    #[test]
    fn a_caller_over_the_limit_stays_limited() {
        let limiter = RateLimiter::with_capacity(2, LONG, 100);
        let key = anonymous_key(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
        assert!(limiter.check(&key));
        assert!(limiter.check(&key));
        assert!(!limiter.check(&key));
    }

    #[test]
    fn a_window_that_has_expired_starts_again() {
        let limiter = RateLimiter::with_capacity(1, Duration::from_millis(20), 100);
        assert!(limiter.check("t:abc"));
        assert!(!limiter.check("t:abc"));
        std::thread::sleep(Duration::from_millis(40));
        assert!(limiter.check("t:abc"));
    }
}
