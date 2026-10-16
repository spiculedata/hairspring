//! Deterministic fuzz-style tests: the parser and folder read untrusted guest output and must never panic, whatever
//! the bytes, and their counters must stay coherent.

mod common;

use common::*;
use hairspring_props::limits::*;
use hairspring_props::*;

/// xorshift64*: a tiny seeded generator so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn seed_lines() -> Vec<Vec<u8>> {
    let all = [
        hello("p1"),
        catalog("p1", "a.b", AssertKind::Always),
        catalog("p1", "c", AssertKind::Unreachable),
        assert_line("p1", "a.b", 3, 2, Some(("w-0", 4)), Some(("w-1", 1))),
        work("p1", 12),
        end("p1"),
        ev(r#"{"v":1,"kind":"eval","id":"e","type":"sometimes","ok":false}"#),
    ]
    .concat();
    all.lines().map(|l| l.as_bytes().to_vec()).collect()
}

fn mutate(rng: &mut Rng, line: &[u8]) -> Vec<u8> {
    let mut out = line.to_vec();
    for _ in 0..=rng.below(4) {
        if out.is_empty() {
            break;
        }
        let at = rng.below(out.len());
        match rng.below(6) {
            0 => out[at] = rng.next() as u8,
            1 => {
                out.remove(at);
            }
            2 => out.insert(at, rng.next() as u8),
            3 => out.truncate(at),
            4 => out.insert(at, b'"'),
            _ => {
                let dup = out[at..].to_vec();
                out.extend(dup);
            }
        }
    }
    out
}

#[test]
fn mutated_valid_lines_never_panic_and_stay_within_limits() {
    let seeds = seed_lines();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let base = &seeds[rng.below(seeds.len())];
        let line = mutate(&mut rng, base);
        if let LineOutcome::Event { event, .. } = parse_line(&line) {
            assert!(line.len() <= MAX_LINE_BYTES);
            assert_event_within_limits(&event);
        }
    }
}

fn assert_event_within_limits(event: &Event) {
    match event {
        Event::Catalog(c) => {
            assert!(c.id.len() <= MAX_ID_BYTES && !c.id.is_empty());
            assert!(c.msg.len() <= MAX_MSG_BYTES);
            assert_eq!(c.must_hit, c.kind.must_hit());
        }
        Event::Assert(a) => {
            assert_eq!(a.trues.checked_add(a.falses), Some(a.hits));
            assert!(a.id.len() <= MAX_ID_BYTES);
        }
        Event::End(e) => {
            assert!(e.threads.len() <= MAX_THREADS_PER_PROC);
            assert!(e.proc.len() <= MAX_PROC_BYTES);
        }
        Event::Hello(h) => assert!(h.proc.len() <= MAX_PROC_BYTES),
        Event::Work(w) => assert!(w.proc.len() <= MAX_PROC_BYTES),
        Event::Eval(e) => assert!(e.id.len() <= MAX_ID_BYTES),
    }
}

#[test]
fn random_bytes_after_the_prefix_never_panic() {
    let mut rng = Rng(42);
    for _ in 0..20_000 {
        let len = rng.below(300);
        let mut line = EVENT_PREFIX.as_bytes().to_vec();
        line.extend((0..len).map(|_| rng.next() as u8));
        let _ = parse_line(&line);
    }
}

#[test]
fn random_json_shaped_noise_never_panics() {
    let alphabet: &[u8] = br#"{}[]:,"\0123456789.-eEtruefalsnv kind proc id type hits ops"#;
    let mut rng = Rng(7);
    for _ in 0..20_000 {
        let len = rng.below(200);
        let mut line = EVENT_PREFIX.as_bytes().to_vec();
        line.extend((0..len).map(|_| alphabet[rng.below(alphabet.len())]));
        let _ = parse_line(&line);
    }
}

#[test]
fn arbitrary_prefixes_and_truncations_of_every_seed_line_are_safe() {
    for base in seed_lines() {
        for cut in 0..=base.len() {
            let _ = parse_line(&base[..cut]);
        }
    }
}

#[test]
fn hostile_shapes_are_rejected_not_crashed_on() {
    let big_num = format!(
        r#"{{"v":1,"kind":"work","proc":"p","ops":{}}}"#,
        "9".repeat(3000)
    );
    let many_keys = {
        let keys: String = (0..400).map(|i| format!(r#","k{i}":{i}"#)).collect();
        format!(r#"{{"v":1,"kind":"work","proc":"p","ops":1{keys}}}"#)
    };
    let cases = [
        big_num,
        many_keys,
        r#"{"v":1,"kind":"work","proc":"p","ops":1e999}"#.to_string(),
        r#"{"v":1e0,"kind":"work"}"#.to_string(),
        r#"{"v":1,"kind":"work","proc":"\ud800","ops":1}"#.to_string(),
        r#"{"v":1,"kind":"work","proc":"p\u0000","ops":1}"#.to_string(),
        "{".repeat(5000),
        "[".repeat(5000),
        String::from("\"unterminated"),
        String::new(),
    ];
    for c in cases {
        let mut line = EVENT_PREFIX.as_bytes().to_vec();
        line.extend_from_slice(c.as_bytes());
        assert!(
            !matches!(parse_line(&line), LineOutcome::Event { .. }),
            "{}",
            &c[..c.len().min(60)]
        );
    }
}

#[test]
fn fold_over_random_mutated_streams_keeps_counters_coherent() {
    let seeds = seed_lines();
    let mut rng = Rng(2026);
    for round in 0..300 {
        let mut out: Vec<u8> = Vec::new();
        for _ in 0..rng.below(60) {
            let base = &seeds[rng.below(seeds.len())];
            let line = if rng.below(3) == 0 {
                mutate(&mut rng, base)
            } else {
                base.clone()
            };
            out.extend_from_slice(&line);
            out.push(b'\n');
        }
        let t = TrialEvents::from_output(&out).fold();
        assert!(t.procs.len() <= MAX_PROCS_PER_TRIAL, "round {round}");
        assert!(t.health.event_bytes as usize <= MAX_EVENT_BYTES_PER_TRIAL);
        for p in &t.properties {
            assert!(
                p.hits >= p.trues && p.hits >= p.falses,
                "round {round}: {p:?}"
            );
        }
        let run = RunProps::new().add(round, &t);
        let _ = run.finish();
        let _ = t.verdict();
    }
}

#[test]
fn large_hostile_output_is_processed_in_bounded_event_memory() {
    // 5 MB of event-prefixed junk: everything past the budget is dropped and counted.
    let junk = format!("HAIRSPRING_EVENT {}\n", "x".repeat(1000));
    let out = junk.repeat(5_000);
    let t = TrialEvents::from_output(out.as_bytes()).fold();
    assert!(t.health.over_limit > 0);
    assert!(t.health.event_bytes as usize <= MAX_EVENT_BYTES_PER_TRIAL);
    assert!(t.is_degraded());
}

#[test]
fn a_single_enormous_line_is_one_too_long_drop() {
    let mut out = EVENT_PREFIX.as_bytes().to_vec();
    out.extend(std::iter::repeat_n(b'a', 20 * 1024 * 1024));
    let t = TrialEvents::from_output(&out).fold();
    assert_eq!(t.health.too_long, 1);
}
