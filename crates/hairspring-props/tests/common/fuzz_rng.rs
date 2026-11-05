//! The seeded generator and case-count settings shared by the generated-input tests.
#![allow(dead_code)]

/// xorshift64*: small, seeded, and good enough to cover shapes rather than to be random.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// How many cases each property runs: `HAIRSPRING_FUZZ_CASES` when set, otherwise `default`.
pub fn cases(default: usize) -> usize {
    std::env::var("HAIRSPRING_FUZZ_CASES")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Runs one generated case. A failing assertion inside `body` is re-raised with the case number, the seed the case was
/// built from and the base seed, so any failure can be repeated from the message alone.
pub fn check_case(case: usize, seed: u64, body: impl FnOnce()) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    if let Err(payload) = result {
        let reason = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "a panic with no text".to_string());
        panic!(
            "case {case}, seed {seed} (HAIRSPRING_FUZZ_SEED={}): {reason}",
            base_seed()
        );
    }
}

/// A base seed from `HAIRSPRING_FUZZ_SEED`, so a long run can be repeated with fresh inputs.
pub fn base_seed() -> u64 {
    std::env::var("HAIRSPRING_FUZZ_SEED")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}
