//! The coverage series as stored with a finished run: encoded when the job finishes, checked when it is read.
//!
//! What is stored is derived data, so a row that cannot be used (damaged, written for another run, or too large to be
//! one this build wrote) is never an error for a reader: it is ignored and the series is worked out again.
use crate::view::Coverage;
use hairspring_store::MAX_COVERAGE_JSON_BYTES;

/// The series as the JSON the store keeps, or `None` when it would not fit the store's limit (the reader then works
/// the series out again).
pub(crate) fn encode(series: &Coverage) -> Option<String> {
    let json = serde_json::to_string(series).ok()?;
    (json.len() <= MAX_COVERAGE_JSON_BYTES).then_some(json)
}

/// The stored series of run `run_id`, or `None` when the stored text is not a series of that run.
pub(crate) fn decode(json: &str, run_id: &str) -> Option<Coverage> {
    if json.len() > MAX_COVERAGE_JSON_BYTES {
        return None;
    }
    let series: Coverage = serde_json::from_str(json).ok()?;
    (series.run_id == run_id && series.points.len() <= crate::query::MAX_COVERAGE_POINTS)
        .then_some(series)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::CoveragePoint;

    fn series(run_id: &str, points: usize) -> Coverage {
        Coverage {
            run_id: run_id.to_owned(),
            trials: points as u64,
            downsampled: false,
            points: (0..points as u64)
                .map(|i| CoveragePoint {
                    seq: i,
                    ids_hit: i,
                    ids_true: i / 2,
                    schedules: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn a_series_round_trips_through_its_stored_form() {
        let s = series("r1", 5);
        let back = decode(&encode(&s).unwrap(), "r1").unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&s).unwrap()
        );
    }

    #[test]
    fn a_full_size_series_fits_the_stores_limit() {
        let s = series("r1", crate::query::MAX_COVERAGE_POINTS);
        assert!(encode(&s).is_some());
    }

    #[test]
    fn a_series_too_large_for_the_store_is_not_encoded() {
        let s = series("r1", 40_000);
        assert!(encode(&s).is_none());
    }

    #[test]
    fn text_that_is_not_a_series_of_this_run_is_not_decoded() {
        let other = encode(&series("r2", 3)).unwrap();
        assert!(decode(&other, "r1").is_none(), "another run's series");
        assert!(decode("not json", "r1").is_none());
        assert!(decode("", "r1").is_none());
        assert!(
            decode("{\"run_id\":\"r1\"}", "r1").is_none(),
            "fields missing"
        );
    }

    #[test]
    fn a_stored_series_with_more_points_than_this_build_writes_is_not_decoded() {
        let crowded =
            serde_json::to_string(&series("r1", crate::query::MAX_COVERAGE_POINTS + 1)).unwrap();
        assert!(decode(&crowded, "r1").is_none());
    }
}
