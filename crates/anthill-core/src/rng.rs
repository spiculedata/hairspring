//! splitmix64: tiny, fast, platform-independent.
const GAMMA: u64 = 0x9E3779B97F4A7C15;

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GAMMA);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish index in `0..n` (modulo bias is irrelevant for exploration). `n` must be > 0.
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "below(0)");
        (self.next_u64() % n as u64) as usize
    }
}
