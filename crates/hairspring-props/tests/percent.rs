use hairspring_props::percent::{lower, nearest, upper};

#[test]
fn an_upper_bound_rounds_up_so_the_shown_figure_is_never_below_the_truth() {
    // 0.9936%: nearest would print 0.99%, which is below the bound.
    assert_eq!(upper(0.009_936_081_944), "1.00%");
    assert_eq!(upper(0.043_165_728_79), "4.32%");
}

#[test]
fn a_lower_bound_rounds_down() {
    assert_eq!(lower(0.005_114_237_79), "0.51%");
    assert_eq!(lower(0.017_876_213_1), "1.78%");
}

#[test]
fn a_figure_that_is_already_two_decimals_is_not_moved_by_float_noise() {
    assert_eq!(upper(0.0149), "1.49%");
    assert_eq!(lower(0.0149), "1.49%");
    assert_eq!(nearest(0.0149), "1.49%");
}

#[test]
fn below_a_hundredth_of_a_percent_the_figure_is_stated_as_less_than_that() {
    assert_eq!(upper(0.000_000_499), "<0.01%");
    assert_eq!(lower(0.000_05), "<0.01%");
    assert_eq!(nearest(0.000_05), "<0.01%");
}

#[test]
fn zero_and_one_are_exact() {
    assert_eq!(upper(0.0), "0.00%");
    assert_eq!(lower(0.0), "0.00%");
    assert_eq!(nearest(0.0), "0.00%");
    assert_eq!(upper(1.0), "100.00%");
    assert_eq!(lower(1.0), "100.00%");
}

#[test]
fn out_of_range_input_is_clamped() {
    assert_eq!(upper(1.5), "100.00%");
    assert_eq!(lower(-0.5), "0.00%");
    assert_eq!(nearest(f64::NAN), "0.00%");
}

#[test]
fn an_invalid_bound_never_looks_tighter_than_it_is() {
    assert_eq!(upper(f64::NAN), "100.00%");
    assert_eq!(lower(f64::NAN), "0.00%");
    assert_eq!(upper(f64::INFINITY), "100.00%");
    assert_eq!(lower(f64::NEG_INFINITY), "0.00%");
}

#[test]
fn the_shown_upper_bound_is_never_below_the_value_and_the_lower_never_above() {
    for i in 0..=2_000u32 {
        let x = f64::from(i) / 2_000.0 * 0.05 + f64::from(i % 7) * 1.3e-6;
        let shown = |s: String| {
            s.trim_start_matches('<')
                .trim_end_matches('%')
                .parse::<f64>()
        };
        let up = shown(upper(x)).unwrap() / 100.0;
        assert!(up + 1e-9 >= x, "{x}: upper shown {up}");
        let low = lower(x);
        if low.starts_with('<') {
            assert!(x < 1e-4, "{x}: lower shown {low}");
        } else {
            let lo = shown(low).unwrap() / 100.0;
            assert!(lo <= x + 1e-9, "{x}: lower shown {lo}");
        }
    }
}
