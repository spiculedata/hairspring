//! The constants of the contract and the three modes chosen by `argv[1]`.

pub const THREADS: u64 = 4;
pub const ITERATIONS: u64 = 1_000;
/// Busy iterations between the load and the store.
pub const WINDOW: u64 = 1_000;
/// The window of the `wide` arm (`WINDOW` times ten).
pub const WINDOW_WIDE: u64 = 10_000;

pub const USAGE: &str = "usage: at-rust buggy|fixed|wide";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Buggy,
    Fixed,
    Wide,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Buggy => "buggy",
            Mode::Fixed => "fixed",
            Mode::Wide => "wide",
        }
    }

    /// Busy iterations in the window of this mode.
    pub fn window(self) -> u64 {
        match self {
            Mode::Wide => WINDOW_WIDE,
            Mode::Buggy | Mode::Fixed => WINDOW,
        }
    }

    /// Whether each iteration holds the mutex.
    pub fn is_locked(self) -> bool {
        self == Mode::Fixed
    }
}

/// The mode named by the arguments after the program name; exactly one argument is accepted.
pub fn parse(args: &[String]) -> Result<Mode, &'static str> {
    match args {
        [only] => [Mode::Buggy, Mode::Fixed, Mode::Wide]
            .into_iter()
            .find(|m| m.name() == only)
            .ok_or(USAGE),
        _ => Err(USAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn each_name_selects_its_mode() {
        assert_eq!(parse(&args(&["buggy"])), Ok(Mode::Buggy));
        assert_eq!(parse(&args(&["fixed"])), Ok(Mode::Fixed));
        assert_eq!(parse(&args(&["wide"])), Ok(Mode::Wide));
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        for bad in [
            args(&[]),
            args(&["Buggy"]),
            args(&[""]),
            args(&["buggy", "fixed"]),
            args(&["--switch=1.0"]),
        ] {
            assert_eq!(parse(&bad), Err(USAGE), "{bad:?}");
        }
    }

    #[test]
    fn names_round_trip() {
        for mode in [Mode::Buggy, Mode::Fixed, Mode::Wide] {
            assert_eq!(parse(&args(&[mode.name()])), Ok(mode));
        }
    }

    #[test]
    fn wide_is_ten_times_the_window_and_the_others_use_the_plain_window() {
        assert_eq!(Mode::Buggy.window(), 1_000);
        assert_eq!(Mode::Fixed.window(), 1_000);
        assert_eq!(Mode::Wide.window(), 10_000);
        assert_eq!(WINDOW_WIDE, WINDOW * 10);
    }

    #[test]
    fn only_the_fixed_mode_holds_the_lock() {
        assert!(Mode::Fixed.is_locked());
        assert!(!Mode::Buggy.is_locked());
        assert!(!Mode::Wide.is_locked());
    }
}
