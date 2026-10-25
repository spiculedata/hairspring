//! The `--seeds a..b` argument of a sweep: the seeds a..b with b left out, so `7..10` is the three trials at seeds 7, 8
//! and 9, as a Rust range reads.
use hairspring_engine::config::MAX_TRIALS_CAP;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedRange {
    pub start: u64,
    /// The first seed not run.
    pub end: u64,
}

impl SeedRange {
    /// How many trials the range names.
    pub fn trials(&self) -> u64 {
        self.end - self.start
    }
}

impl FromStr for SeedRange {
    type Err = String;

    fn from_str(text: &str) -> Result<SeedRange, String> {
        let form = || {
            format!(
                "`{}` is not a seed range; write it as FIRST..END, for example 100..200",
                text.chars().take(40).collect::<String>()
            )
        };
        let (first, end) = text.split_once("..").ok_or_else(form)?;
        let number = |s: &str| s.parse::<u64>().map_err(|_| form());
        let range = SeedRange {
            start: number(first)?,
            end: number(end)?,
        };
        if range.end <= range.start {
            return Err(format!(
                "the seed range {text} is empty; END must be above FIRST (END itself is not run)"
            ));
        }
        if range.trials() > MAX_TRIALS_CAP {
            return Err(format!(
                "the seed range {text} names {} trials; a run may have at most {MAX_TRIALS_CAP}",
                range.trials()
            ));
        }
        Ok(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_names_the_seeds_from_its_start_up_to_but_not_including_its_end() {
        let r: SeedRange = "7..10".parse().unwrap();
        assert_eq!((r.start, r.end, r.trials()), (7, 10, 3));
    }

    #[test]
    fn the_largest_allowed_range_is_accepted_and_one_more_is_not() {
        assert!(format!("5..{}", 5 + MAX_TRIALS_CAP)
            .parse::<SeedRange>()
            .is_ok());
        let err = format!("5..{}", 6 + MAX_TRIALS_CAP)
            .parse::<SeedRange>()
            .unwrap_err();
        assert!(err.contains("at most"), "{err}");
    }

    #[test]
    fn an_empty_reversed_or_malformed_range_is_refused_with_a_reason() {
        for bad in [
            "7..7", "10..7", "abc", "1-5", "..5", "5..", "1..2..3", "-1..5", "1..x", "",
        ] {
            assert!(bad.parse::<SeedRange>().is_err(), "{bad}");
        }
        assert!("7..7".parse::<SeedRange>().unwrap_err().contains("empty"));
        assert!("abc"
            .parse::<SeedRange>()
            .unwrap_err()
            .contains("FIRST..END"));
    }

    #[test]
    fn a_range_near_the_top_of_the_seed_space_does_not_overflow() {
        let r: SeedRange = format!("{}..{}", u64::MAX - 3, u64::MAX).parse().unwrap();
        assert_eq!(r.trials(), 3);
    }

    #[test]
    fn the_text_of_a_refused_range_is_bounded() {
        let err = "x".repeat(500).parse::<SeedRange>().unwrap_err();
        assert!(err.len() < 200, "{}", err.len());
    }
}
