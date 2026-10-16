//! The schedule fingerprints a run has seen, kept to a fixed number so a long run's memory does not grow with it.
use std::collections::HashSet;

/// Most distinct fingerprints one run remembers. Each is 16 hex digits, so this is about 100 KiB; a run is limited
/// to 10,000 trials, so a run that reaches it was exploring nearly as many schedules as it had trials.
pub const MAX_SCHEDULES_PER_RUN: usize = 4096;

#[derive(Debug, Clone)]
pub struct ScheduleSet {
    cap: usize,
    seen: HashSet<String>,
    /// Trials whose fingerprint was not among the remembered ones once the set was full. Some of them may share a
    /// fingerprint with each other, which is why the true number of distinct schedules is only bounded from below.
    unremembered: u64,
}

impl Default for ScheduleSet {
    fn default() -> Self {
        ScheduleSet::with_cap(MAX_SCHEDULES_PER_RUN)
    }
}

impl ScheduleSet {
    pub fn with_cap(cap: usize) -> ScheduleSet {
        ScheduleSet {
            cap,
            seen: HashSet::new(),
            unremembered: 0,
        }
    }

    pub fn insert(&mut self, fingerprint: &str) {
        if self.seen.contains(fingerprint) {
            return;
        }
        match self.seen.len() < self.cap {
            true => {
                self.seen.insert(fingerprint.to_string());
            }
            false => self.unremembered = self.unremembered.saturating_add(1),
        }
    }

    /// How many distinct schedules the run is known to have run: exact until the cap is reached, then a lower bound.
    pub fn distinct(&self) -> u64 {
        self.seen.len() as u64
    }

    /// True when a fingerprint arrived that the full set could not take, so [`ScheduleSet::distinct`] undercounts.
    pub fn is_capped(&self) -> bool {
        self.unremembered > 0
    }

    /// Trials that ran a schedule the set could not remember.
    pub fn unremembered(&self) -> u64 {
        self.unremembered
    }

    /// `"12 distinct schedules"`, or `"at least 4096 distinct schedules"` once the cap has cut the count short.
    pub fn describe(&self) -> String {
        let n = self.distinct();
        let unit = if n == 1 { "schedule" } else { "schedules" };
        match self.is_capped() {
            true => format!("at least {n} distinct {unit}"),
            false => format!("{n} distinct {unit}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(set: &mut ScheduleSet, fingerprints: &[&str]) {
        for fp in fingerprints {
            set.insert(fp);
        }
    }

    #[test]
    fn below_the_cap_the_count_is_exact_and_not_marked() {
        let mut set = ScheduleSet::with_cap(5);
        fill(&mut set, &["a", "b", "a", "c", "b"]);
        assert_eq!(set.distinct(), 3);
        assert!(!set.is_capped());
        assert_eq!(set.describe(), "3 distinct schedules");
    }

    #[test]
    fn exactly_at_the_cap_is_still_exact() {
        let mut set = ScheduleSet::with_cap(3);
        fill(&mut set, &["a", "b", "c", "a", "b", "c"]);
        assert_eq!(set.distinct(), 3);
        assert!(!set.is_capped());
        assert_eq!(set.describe(), "3 distinct schedules");
    }

    #[test]
    fn past_the_cap_the_count_stops_growing_and_says_at_least() {
        let mut set = ScheduleSet::with_cap(3);
        fill(&mut set, &["a", "b", "c", "d", "e", "f"]);
        assert_eq!(set.distinct(), 3, "only the cap's worth is remembered");
        assert!(set.is_capped());
        assert_eq!(set.unremembered(), 3);
        assert_eq!(set.describe(), "at least 3 distinct schedules");
    }

    #[test]
    fn a_remembered_schedule_seen_again_after_the_cap_is_not_an_overflow() {
        let mut set = ScheduleSet::with_cap(2);
        fill(&mut set, &["a", "b", "a", "b", "a"]);
        assert!(!set.is_capped());
        assert_eq!(set.unremembered(), 0);
    }

    #[test]
    fn memory_is_bounded_by_the_cap() {
        let mut set = ScheduleSet::with_cap(100);
        for i in 0..10_000 {
            set.insert(&format!("{i:016x}"));
        }
        assert_eq!(set.seen.len(), 100);
        assert_eq!(set.unremembered(), 9_900);
    }

    #[test]
    fn one_schedule_is_singular() {
        let mut set = ScheduleSet::with_cap(5);
        set.insert("a");
        assert_eq!(set.describe(), "1 distinct schedule");
    }
}
