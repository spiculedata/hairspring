#!/usr/bin/env python3
# glg_eval.py : the pre-registered rules of the GLG spec (sections 6 and 9). Nothing here is tuned after seeing data. Every number is copied from GLIBC-CONDLOCK-DESIGN.
# The rules are applied per PART (A or P), the part of the run being the last argument or the third field of out/args.txt:
#   PART A: steps tp (arm C only), g0 (arms u and c), dposh, dstallh, then the G2A runs (recorded, never a gate); PART P: steps tp (arm P only), g0 (arm p), g1, g2c.
#   glg_eval.py gate STEP RUNS_TSV RND G0S PART   prints one line: "PASS <step> <labels>", "STOP <code>" or "MISSING <step>"   (glg.sh, between steps; STEP is tp g0 dposh dstallh g1 g2c)
#   glg_eval.py first RUNS_TSV G0S PART           prints ALLBAD when the first G0 seed's runs (3 per arm of the part) all failed to complete, else OK   (glg.sh stops spending runs on ALLBAD)
#   glg_eval.py skidnext RUNS_TSV ARM SEED G0S PART   prints "RUN <spare seed>" when the pair (ARM, SEED) is to be replaced by the SKID_OVERSHOOT rule and the spare's runs are not yet made, else DONE
#   glg_eval.py pause RUNS_TSV G0S NOW NEWBY PART prints "OK need=<s> left=<s>" or "PAUSE need=<s> left=<s>": the projected-time check after G0 (design 7.4), per part
# The stderr of a run is read (out/<id>.stderr next to RUNS_TSV) ONLY to recognise the SKID_OVERSHOOT signature of a Hermit failure; nothing else reads it.
#   glg_eval.py verdict RUN_DIR              prints the tables and "OUTCOME <code>"   (run on the Mac over the archive copy of $R)
#   glg_eval.py selftest                     exhaustive and sample tests, no files written (also run on the box as gate EVAL-SELFTEST)
import math
import os
import re
import sys

COLS = ("id step arm k seed rep rc ms cls subtype prop_false stall_hit applied hit_also libc_ok rounds signals blocked1 maxb maxstag ticks spanned nout ct wt "
        "stdout_sha fp syscalls virt_ns turns dump").split()
STR_COLS = {"id", "step", "arm", "cls", "subtype", "stdout_sha", "fp"}
TP_COLS = "id case arm rc ms virt_ns turns ok".split()
TP_STR = {"id", "case", "arm"}
Z = 1.959964                                   # 95% two-sided (design 8)
ARMS = ("u", "c", "p")                         # A-u, A-c, P-u
RATED = ("NOHIT", "HIT", "STALL")              # design 5.2: rated = HIT + NOHIT + STALL
COMPLETE = RATED                               # a run that finished as a legal run (a hit is not a determinism failure)
CLASSES = ("NOHIT", "HIT", "STALL", "TIMEOUT", "HERMIT-FAIL", "DEADLOCK-REPORT", "CRASH", "HARNESS-FAIL")
FAILED = ("HERMIT-FAIL", "CRASH", "HARNESS-FAIL", "DEADLOCK-REPORT")      # real failures to complete (a TIMEOUT is slowness, not in this list)
STEPS = ("tp", "g0", "dposh", "dstallh", "g1", "g2c")
PART_STEPS = {"A": ("tp", "g0", "dposh", "dstallh"), "P": ("tp", "g0", "g1", "g2c")}      # the gated steps of each part (PART A also makes the 20 G2A runs, recorded, never gated)
PART_ARMS = {"A": ("u", "c"), "P": ("p",)}                                                  # the G0 arms of each part
PARTS = ("A", "P")
FIRST = {"tp": "TP", "g0": "G0", "dposh": "DPH", "dstallh": "DSH", "g1": "G1", "g2c": "G2C"}
LAUNCHES = {319101: 500, 319511: 250, 319521: 125, 319531: 60, 319541: 30}   # G0 block -> RND of that launch (spec 6.4); 319551 is reserved, not used
RND_LADDER = (500, 250, 125, 60, 30)           # design 7.4: halve, never raise, floor 30
SPARE_PAIRS = {319101: (319561, 319562), 319511: (319563, 319564), 319521: (319565, 319566), 319531: (319567, 319568), 319541: (319569, 319570)}   # two per launch
SKID_MAX_REPLACEMENTS = 2                      # G0: at most 2 SKID_OVERSHOOT replacements per launch (GOG 5.3a)
SKID_STEP_CAP = 2                              # the other steps: at most 2 SKID_OVERSHOOT runs per step are excluded, a third is INCOMPLETE
A_MEDIAN_LIMIT_MS = 2700                       # design 7.4: C1 is 4,000 runs in a 3 h cap
P_MEDIAN_LIMIT_MS = 6000                       # design 7.4: p <= 6.0 s
KILL_MS = 30000                                # design 7.4: over 30 s at the floor is a kill
STAG_MAX = 3                                   # design 2.4 G0w: at most 3 stagnant ticks in healthy runs (margin >= 3.3x against Q = 10)
TICKS_MIN = 3                                  # design 2.4 / 13c: mean ticks per healthy run at least 3, or the shortfall is reported
MIN_DISTINCT_FP = 3                            # design 7.3 G0d
MIN_P_HITS = 3                                 # design 7.3 G1 and G2c: at least 3 of 200
STALLQ_LOW = 3                                 # design 7.3: P-u fails the 3/200 rule with STALL-Q >= 3: G1-PU-LOW-STALLQ
PLANT_WALL_MS = 10000                          # planted runs end by fast-forward: wall under 10 s (the T-probe's own threshold)
APPLIED_PCT = 95                               # design 6.1: applied in at least 95% of runs
FF_WALL_MS, FF_VIRT_NS = 10000, 60000000000    # design 7.3 T-probe (i) and (iii)
PROJECT_P_RUNS, PROJECT_A_RUNS, PROJECT_MARGIN_S = 400, 80, 600     # design 7.4 projected-time check, per part: PART P 400 P runs (G1 200 + G2c 200); PART A 80 A-sized runs (D-pos-H 30, D-stall-H 30, G2a/b 20); 600 s
C1_RUNS_PER_ARM = 2000
DPOS_SEEDS = tuple(range(319601, 319631))
PLANT_KS = (1, 2, 5, 20, 50)
STALL_PLANTS = {"l": (319641, "L"), "b": (319651, "B"), "h": (319661, "M")}      # arm -> (first seed of its 10, expected STALL subtype)
G1_SEEDS = tuple(range(319201, 319401))
G2A_SEEDS = tuple(range(319401, 319411))
TP_NEED_PART = {"A": (("i", "c"), ("iii", "c")), "P": (("i", "p"), ("iii", "p"))}      # PART A rechecks the fast-forward on arm C only; the arm P recheck is PART P's
SYSCALL_DIFF_FORMULA = 33                      # design 5.3: 1+1+9+1+9+1+T+1 with T = 10 reporting threads (main, the watchdog and 8 workers each call hs_thread: src/main.c:27, cl_watchdog.c:127, cl_worker.c:14; the SDK end line of every X1b run says "threads":10); the formula is the design's HYPOTHESIS, recorded, never a gate


def wilson(k, n, z=Z):
    if n <= 0:
        return None, None
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def zero_bound(n, alpha=0.05):
    return 1 - alpha ** (1.0 / n) if n > 0 else None


def pct(x, digits=4):
    return "n/a" if x is None else ("%." + str(digits) + "f%%") % (100 * x)


def rate_text(k, n):
    lo, hi = wilson(k, n)
    zb = " exact-zero-upper=%s" % pct(zero_bound(n)) if k == 0 and n > 0 else ""
    return "%d/%d wilson95=[%s, %s]%s" % (k, n, pct(lo), pct(hi), zb)


def fisher_two_sided(k1, n1, k2, n2):
    """Two-sided exact p-value (the sum of the tables at most as likely as the observed one) for k1 of n1 against k2 of n2."""
    tot, n = k1 + k2, n1 + n2
    lo, hi = max(0, tot - n2), min(n1, tot)
    pr = lambda x: math.comb(n1, x) * math.comb(n2, tot - x) / math.comb(n, tot)
    p0 = pr(k1)
    return min(1.0, sum(p for p in (pr(x) for x in range(lo, hi + 1)) if p <= p0 * (1 + 1e-9)))


def median(xs):
    s = sorted(xs)
    n = len(s)
    if n == 0:
        return None
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2


def read_text(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def parse_rows(text):
    """runs.tsv text -> list of dicts. Integer columns become int; a field that is not an integer (for example applied '-') becomes -1."""
    rows = []
    for line in text.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) != len(COLS):
            continue
        r = {}
        for name, v in zip(COLS, f):
            if name in STR_COLS:
                r[name] = v
            else:
                try:
                    r[name] = int(v)
                except ValueError:
                    r[name] = -1
        rows.append(r)
    return rows


def parse_tp(text):
    rows = []
    for line in text.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) != len(TP_COLS):
            continue
        r = {}
        for name, v in zip(TP_COLS, f):
            if name in TP_STR:
                r[name] = v
            else:
                try:
                    r[name] = int(v)
                except ValueError:
                    r[name] = -1
        rows.append(r)
    return rows


def consistent(r):
    """The exit status and the two site flags agree with the class (condlock exits 0, 3 on a property hit, 4 on a stall)."""
    c = r["cls"]
    if c == "NOHIT":
        return r["rc"] == 0 and r["prop_false"] == 0 and r["stall_hit"] == 0
    if c == "HIT":
        return r["rc"] == 3 and r["prop_false"] == 1
    if c == "STALL":
        return r["rc"] == 4 and r["stall_hit"] == 1
    return True


def klass(r):
    """The class of one run as counted. classify.sh's class is taken (per site, never from an engine verdict); an unknown class, a wrong libc line, a stdout that is
    not one line, or a class that disagrees with the exit status and the site flags is HARNESS-FAIL."""
    c = r["cls"]
    if c in ("TIMEOUT", "CRASH", "HERMIT-FAIL", "DEADLOCK-REPORT", "HARNESS-FAIL"):
        return c
    if c not in CLASSES:
        return "HARNESS-FAIL"
    if c in RATED and (r["libc_ok"] != 1 or r["nout"] != 1 or not consistent(r)):
        return "HARNESS-FAIL"
    return c


def base_subtype(s):
    return s.split("+")[0] if s else "-"


def sel(rows, step, **kw):
    return [r for r in rows if r["step"] == step and all(r[k] == v for k, v in kw.items())]


def seedlist(g0s):
    return list(range(g0s, g0s + 10))


# ---------------------------------------------------------------- the SKID_OVERSHOOT rule (GOG 5.3a, strict signature, bounded replacement, ledger)
SKID_LINE = re.compile(r"^HERMIT_SKID_OVERSHOOT rcb_actual=\d+ rcb_target=\d+ skid_margin=\d+ overshoot=\d+$")
SKID_PANIC_AT = "panicked at detcore/src/lib.rs:348"
SKID_INTERNAL = "HERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)"
SKID_FOREIGN = ("HERMIT_CLEANUP_UNCONFIRMED", "tool_global.rs:1619", "Usage: hermit", "Deadlock detected")


def no_stderr(run_id):
    return ""


def stderr_reader(out_dir):
    return lambda run_id: read_text(os.path.join(out_dir, run_id + ".stderr"))


def out_dir_of(runs_tsv):
    return os.path.join(os.path.dirname(os.path.abspath(runs_tsv)), "out")


def spares_for(g0s):
    return SPARE_PAIRS.get(g0s, SPARE_PAIRS[319101])


def skid_run(r, text):
    """True when the run is a Hermit failure of the single class SKID_OVERSHOOT: exit status 125, class HERMIT-FAIL, a HERMIT_SKID_OVERSHOOT rcb line, and nothing else
    Hermit-marked in stderr (the panic only at detcore/src/lib.rs:348, the internal failure only a container-child exit 101, a task panic only with the skid message,
    no cleanup-unconfirmed, no usage text, no deadlock report). The decision reads the exit status and the stderr text only: never a hit, a fingerprint or a stdout."""
    if r["rc"] != 125 or r["cls"] != "HERMIT-FAIL":
        return False
    lines = text.splitlines()
    if not any(SKID_LINE.match(ln) for ln in lines):
        return False
    for ln in lines:
        if any(m in ln for m in SKID_FOREIGN):
            return False
        if "panicked at" in ln and SKID_PANIC_AT not in ln:
            return False
        if "HERMIT_INTERNAL_FAILURE" in ln and SKID_INTERNAL not in ln:
            return False
        if "HERMIT_TASK_PANIC" in ln and "message=HERMIT_SKID_OVERSHOOT" not in ln:
            return False
    return True


def pair_state(rs, se):
    """One (arm, seed) pair of 3 runs: MISSING (not 3 rows); OK (every run completed); SKID (every run that did not complete is a skid_run); OTHER (any other defect, a timeout included)."""
    if len(rs) != 3:
        return "MISSING"
    bad = [r for r in rs if klass(r) not in COMPLETE]
    if not bad:
        return "OK"
    return "SKID" if all(skid_run(r, se(r["id"])) for r in bad) else "OTHER"


def g0_effective(rows, g0s, se, arms=("u", "c")):
    """The driver's order (seed-major, then arm, then the replacement chain) replayed on the rows, for the arms of the part (A: u and c; P: p).
    -> (eff, notes, pending, nondet): eff[(arm, seed)] = (the runs of the pair that counts, the seed they ran with); notes = one line per replacement;
    pending[(arm, seed)] = the spare seed whose runs have been decided but not yet made; nondet = one line per replaced pair whose completed runs disagree.
    A replaced pair's rows stay raw in runs.tsv and are in no other list (G0d, G0c, medians, evidence); two or more completed runs of it that differ are G0-NONDET."""
    g0, g0r = sel(rows, "G0"), sel(rows, "G0R")
    spares, used = list(spares_for(g0s)), 0
    eff, notes, pending, nondet = {}, [], {}, []
    for s in seedlist(g0s):
        for arm in arms:
            cur, src = s, g0
            while True:
                rs = [r for r in src if r["arm"] == arm and r["seed"] == cur]
                st = pair_state(rs, se)
                if st != "SKID":
                    break
                if used >= SKID_MAX_REPLACEMENTS:
                    notes.append("SKID_OVERSHOOT: A-%s seed %d is not replaced, the %d replacements of this launch are used (G0-INCOMPLETE)" % (arm, cur, SKID_MAX_REPLACEMENTS))
                    break
                nxt = spares[used]
                used += 1
                done = [r for r in rs if klass(r) in COMPLETE]
                so_d, fp_d = len({r["stdout_sha"] for r in done}) > 1, len({r["fp"] for r in done}) > 1
                notes.append("SKID_OVERSHOOT replacement %d of %d: %s seed %d -> spare seed %d; raw rows kept, excluded from scoring: %s; its %d completed runs: stdout %s, fingerprint %s" % (
                    used, SKID_MAX_REPLACEMENTS, arm, cur, nxt, ", ".join(r["id"] for r in rs), len(done), "DIFFER" if so_d else "equal", "DIFFER" if fp_d else "equal"))
                if so_d or fp_d:
                    nondet.append("G0-NONDET: replaced pair %s seed %d: its completed runs differ in %s" % (
                        arm, cur, "stdout and fingerprint" if so_d and fp_d else "stdout" if so_d else "fingerprint"))
                cur, src = nxt, g0r
            if cur != s and st == "MISSING" and not rs:
                pending[(arm, s)] = cur
            eff[(arm, s)] = (rs, cur)
    return eff, notes, pending, nondet


def arm_runs(eff, g0s, arm):
    return [r for s in seedlist(g0s) for r in eff[(arm, s)][0]]


# ---------------------------------------------------------------- timing (a function of the measured wall milliseconds and RND only: never of a class, a hit or a fingerprint)
def timing_a(ms_u, ms_c, rnd):
    """Design 7.4 G0t (A). -> (code or None, medians). Each arm's median over its 30 effective G0 runs (a timeout counted at its measured wall) at most 2,700 ms; over: halve RND (never raise);
    at the floor 30: over 30 s is a kill, otherwise the budget stop (a decision about more machine time is needed)."""
    mu, mc = median(ms_u), median(ms_c)
    over = (mu is not None and mu > A_MEDIAN_LIMIT_MS) or (mc is not None and mc > A_MEDIAN_LIMIT_MS)
    if not over:
        return None, (mu, mc)
    if rnd > RND_LADDER[-1]:
        return "G0T-LOWER-R", (mu, mc)
    if (mu or 0) > KILL_MS or (mc or 0) > KILL_MS:
        return "G0T-KILL", (mu, mc)
    return "G0T-C1-BUDGET", (mu, mc)


def timing_p(ms_p):
    mp = median(ms_p)
    return ("G0T-P-SLOW" if mp is not None and mp > P_MEDIAN_LIMIT_MS else None), mp


def next_rnd(rnd):
    """The RND of the relaunch after G0T-LOWER-R: the next rung of the ladder (halving, never raised)."""
    i = RND_LADDER.index(rnd)
    return RND_LADDER[i + 1] if i + 1 < len(RND_LADDER) else rnd


def arm_evidence(runs):
    """Per arm, over the effective G0 runs: G0w (the healthy runs' stagnation and ticks), the G0c coverage, the 13c evidence columns."""
    healthy = [r for r in runs if klass(r) == "NOHIT"]
    ticks = [r["ticks"] for r in healthy if r["ticks"] >= 0]
    return {
        "runs": len(runs), "healthy": len(healthy), "maxstag": max([r["maxstag"] for r in healthy], default=0),
        "mean_ticks": (sum(ticks) / len(ticks)) if ticks else None,
        "ct_runs": sum(1 for r in runs if r["ct"] > 0), "wt_runs": sum(1 for r in runs if r["wt"] > 0),
        "blocked1_sum": sum(max(r["blocked1"], 0) for r in runs), "spanned_sum": sum(max(r["spanned"], 0) for r in runs),
        "hits": sum(1 for r in runs if klass(r) == "HIT"), "stalls": sum(1 for r in runs if klass(r) == "STALL"),
    }


def short_ticks(ev):
    return ev["mean_ticks"] is None or ev["mean_ticks"] < TICKS_MIN


# ---------------------------------------------------------------- G0 (determinism triples), G0d, G0c, G0w, G0t
def g0_eval(rows, rnd, g0s, se=no_stderr, part="A"):
    """-> (kind, code, notes, info) with kind PASS, STOP or MISSING. On PASS the code is the label string ('' or '+SHORT-TICKS-U...').
    PART A (arms u and c; this is the part that freezes RND): G0-NONDET; G0-ALLBAD; G0-INCOMPLETE (a real failure to complete: a Hermit failure beyond the SKID_OVERSHOOT rule, a crash,
    a harness failure, a deadlock report; a timeout is slowness here); G0-CONTROL-HIT / G0-CONTROL-STALL (arm C); MISSING runs; the timing rule (G0T-LOWER-R, G0T-KILL,
    G0T-C1-BUDGET); a timeout at a fine median (G0-INCOMPLETE); G0W-STAG; G0D-NODIV; G0C-NO-CONTENDED; G0C-NO-SPAN; PASS.
    PART P (arm p, at the RND that part A froze): G0-NONDET; G0-ALLBAD; G0-INCOMPLETE; MISSING runs; a timeout at a fine median (G0-INCOMPLETE); G0T-P-SLOW (a stop for a human
    decision: P's RND is never lowered on its own); G0W-STAG; PASS. No A rule is read in part P and no P rule in part A."""
    arms = PART_ARMS[part]
    eff, notes, _, excl_nondet = g0_effective(rows, g0s, se, arms)
    notes = list(notes) + excl_nondet
    per, runs = {}, {a: arm_runs(eff, g0s, a) for a in arms}
    info = {"eff": eff, "runs": runs, "per": per, "part": part}
    for (arm, s), (rs, cur) in sorted(eff.items(), key=lambda kv: (kv[0][1], ARMS.index(kv[0][0]))):
        if len(rs) != 3:
            per[(arm, s)] = "MISSING"
        elif any(klass(r) not in COMPLETE for r in rs):
            per[(arm, s)] = "INCOMPLETE:" + ",".join(sorted({klass(r) for r in rs if klass(r) not in COMPLETE}))
        elif any(r["fp"] in ("NOSUMMARY", "") for r in rs):
            per[(arm, s)] = "INCOMPLETE:NOSUMMARY"
        else:
            so, fp = len({r["stdout_sha"] for r in rs}) > 1, len({r["fp"] for r in rs}) > 1
            per[(arm, s)] = "DET" if not so and not fp else "NONDET-" + ("both" if so and fp else "stdout" if so else "fp")
    nondet = [k for k, v in per.items() if v.startswith("NONDET")]
    if nondet or excl_nondet:
        return "STOP", "G0-NONDET", notes + ["NONDET pairs: %s" % ", ".join("%s/%d" % k for k in sorted(nondet))], info
    first = [r for a in arms for r in eff[(a, seedlist(g0s)[0])][0]]
    if len(first) == 3 * len(arms) and all(klass(r) not in COMPLETE for r in first):
        return "STOP", "G0-ALLBAD", notes, info
    failed = sorted({"%s/%d:%s" % (k[0], k[1], klass(r)) for k, (rs, _) in eff.items() for r in rs if klass(r) in FAILED})
    if failed:
        return "STOP", "G0-INCOMPLETE", notes + ["failed to complete (not a timeout): " + ", ".join(failed)], info
    nosum = sorted("%s/%d" % k for k, v in per.items() if v == "INCOMPLETE:NOSUMMARY")
    if nosum:
        return "STOP", "G0-INCOMPLETE", notes + ["no Hermit summary (the fingerprint cannot be read): " + ", ".join(nosum)], info
    if part == "A":
        ctl_hit = [r["id"] for r in runs["c"] if klass(r) == "HIT"]
        ctl_stall = [r["id"] for r in runs["c"] if klass(r) == "STALL"]
        if ctl_hit:
            return "STOP", "G0-CONTROL-HIT", notes + ["A-c runs with the property site false: " + ", ".join(ctl_hit)], info
        if ctl_stall:
            return "STOP", "G0-CONTROL-STALL", notes + ["A-c runs ending in STALL: " + ", ".join(ctl_stall)], info
    if any(v == "MISSING" for v in per.values()):
        return "MISSING", "G0", notes, info
    if part == "A":
        code, meds = timing_a([r["ms"] for r in runs["u"]], [r["ms"] for r in runs["c"]], rnd)
        info["medians"] = meds
        notes.append("G0t (A): median A-u %s ms, A-c %s ms, limit %d ms, RND %d -> %s" % (meds[0], meds[1], A_MEDIAN_LIMIT_MS, rnd, code or "within the limit"))
        if code:
            return "STOP", code, notes, info
    timed_out = sorted({"%s/%d" % (k[0], k[1]) for k, (rs, _) in eff.items() for r in rs if klass(r) == "TIMEOUT"})
    if timed_out:
        return "STOP", "G0-INCOMPLETE", notes + ["timeouts at a fine median: " + ", ".join(timed_out)], info
    if part == "P":
        code, mp = timing_p([r["ms"] for r in runs["p"]])
        info["median_p"] = mp
        notes.append("G0t (P): median P-u %s ms, limit %d ms, RND %d (frozen by part A, never lowered here) -> %s" % (mp, P_MEDIAN_LIMIT_MS, rnd, code or "within the limit"))
        if code:
            return "STOP", code, notes, info
    evs = {a: arm_evidence(runs[a]) for a in arms}
    info["ev"] = evs
    over = [a for a in arms if evs[a]["maxstag"] > STAG_MAX]
    if over:
        return "STOP", "G0W-STAG", notes + ["G0w: healthy-run maxstag above %d in %s" % (STAG_MAX, ",".join("A-" + a if a != "p" else "P-u" for a in over))], info
    if part == "A":
        ufp = len({sorted(r["fp"] for r in eff[("u", s)][0])[0] for s in seedlist(g0s)})
        info["distinct_fp"] = ufp
        if ufp < MIN_DISTINCT_FP:
            return "STOP", "G0D-NODIV", notes + ["G0d: %d distinct A-u fingerprints among 10 seeds" % ufp], info
        if evs["u"]["ct_runs"] < 1:
            return "STOP", "G0C-NO-CONTENDED", notes, info
        if evs["u"]["wt_runs"] < 1:
            return "STOP", "G0C-NO-SPAN", notes, info
    labels = "".join("+SHORT-TICKS-%s" % ("P" if a == "p" else a.upper()) for a in arms if short_ticks(evs[a]))
    return "PASS", labels, notes, info


def g0_first(rows, g0s, part="A"):
    first = [r for r in sel(rows, "G0") if r["seed"] == seedlist(g0s)[0]]
    return "ALLBAD" if len(first) == 3 * len(PART_ARMS[part]) and all(klass(r) not in COMPLETE for r in first) else "OK"


def skid_next(rows, arm, seed, g0s, se, part="A"):
    _, _, pending, _ = g0_effective(rows, g0s, se, PART_ARMS[part])
    sp = pending.get((arm, seed))
    return "RUN %d" % sp if sp else "DONE"


def projected_pause(rows, g0s, now, newby, se=no_stderr, part="A"):
    """Design 7.4, per part, against the time left before the no-new-runs line. PART P: 400 x (P-u median) + 600 s (G1 200 runs and G2c 200 runs). PART A: 80 x (the larger of the
    A-u and A-c medians) + 600 s (D-pos-H 30, D-stall-H 30, G2a/b 20 runs; planted runs end earlier, so this is an upper bound). No medians pauses."""
    eff = g0_effective(rows, g0s, se, PART_ARMS[part])[0]
    left = newby - now
    if part == "P":
        p = median([r["ms"] for r in arm_runs(eff, g0s, "p")])
        if p is None:
            return "PAUSE", None, left
        need = PROJECT_P_RUNS * p / 1000.0 + PROJECT_MARGIN_S
    else:
        mu, mc = median([r["ms"] for r in arm_runs(eff, g0s, "u")]), median([r["ms"] for r in arm_runs(eff, g0s, "c")])
        if mu is None or mc is None:
            return "PAUSE", None, left
        need = PROJECT_A_RUNS * max(mu, mc) / 1000.0 + PROJECT_MARGIN_S
    return ("PAUSE" if need > left else "OK"), need, left


# ---------------------------------------------------------------- T-probe recheck
def tp_eval(tp, part="A"):
    need = TP_NEED_PART[part]
    have = {(t["case"], t["arm"]): t for t in tp}
    if not all(k in have for k in need):
        return "MISSING", "TP", ["T-probe rows present: %d of %d" % (sum(1 for k in need if k in have), len(need))]
    notes, bad = [], []
    for k in need:
        t = have[k]
        ok = t["ok"] == 1 and 0 <= t["ms"] < FF_WALL_MS and t["virt_ns"] >= FF_VIRT_NS
        notes.append("tprobe %s on %s: ok=%d wall_ms=%d virt_ns=%d turns=%d -> %s" % (k[0], k[1].upper(), t["ok"], t["ms"], t["virt_ns"], t["turns"], "FF-OK" if ok else "FF-FAIL"))
        if not ok:
            bad.append(k)
    return ("STOP", "TP-FF-FAIL", notes) if bad else ("PASS", "", notes)


# ---------------------------------------------------------------- the planted self-tests D-pos-H and D-stall-H
def planted_split(rs, se):
    """Runs of a planted step -> (used, skid-excluded). A SKID_OVERSHOOT run is excluded (ledgered), never replaced."""
    excl = [r for r in rs if skid_run(r, se(r["id"]))]
    return [r for r in rs if r not in excl], excl


def planted_common(rs, expect, se, code, applied_by_group):
    """Shared checks: all expected seeds present; at most SKID_STEP_CAP skid exclusions; no failure class; k as expected; applied flags consistent."""
    present = {r["seed"]: r for r in rs}
    if not all(s in present for s, _ in expect):
        return "MISSING", None, []
    used, excl = planted_split(rs, se)
    notes = ["%s: %d runs, %d SKID_OVERSHOOT run(s) excluded: %s" % (code, len(rs), len(excl), ", ".join(r["id"] for r in excl) or "none")]
    if len(excl) > SKID_STEP_CAP:
        return "STOP", "%s-INCOMPLETE" % code, notes
    want_k = dict(expect)
    for r in used:
        if klass(r) in FAILED + ("TIMEOUT",) or r["k"] != want_k[r["seed"]]:
            return "STOP", "%s-BAD" % code, notes + ["bad run %s class %s k %d (want %d)" % (r["id"], klass(r), r["k"], want_k[r["seed"]])]
        if r["applied"] not in (0, 1):
            return "STOP", "%s-BAD" % code, notes + ["run %s has no applied flag" % r["id"]]
        if r["applied"] == 0 and klass(r) != "NOHIT":
            return "STOP", "%s-BAD" % code, notes + ["run %s was not applied but is %s" % (r["id"], klass(r))]
    return "OK", used, notes


def dposh_eval(rows, se=no_stderr):
    """Design 6.1: the deaf-condvar plant on arm C. applied in at least 95% of the used runs; every applied run is class HIT (exit 3, property site false) and ends within
    PLANT_WALL_MS (stagnation detected by fast-forward). Subtype bits are reported, not gated."""
    rs = sel(rows, "DPH")
    expect = [(s, PLANT_KS[(s - DPOS_SEEDS[0]) % len(PLANT_KS)]) for s in DPOS_SEEDS]
    st, used, notes = planted_common(rs, expect, se, "DPOSH", None)
    if st != "OK":
        return st, "DPOSH" if st == "MISSING" else used, notes
    ap = [r for r in used if r["applied"] == 1]
    notes.append("D-pos-H: applied %d of %d used runs; HIT in %d; max wall %d ms" % (len(ap), len(used), sum(1 for r in ap if klass(r) == "HIT"), max([r["ms"] for r in ap], default=0)))
    if len(ap) * 100 < APPLIED_PCT * len(used):
        return "STOP", "DPOSH-NOTAPPLIED", notes
    if any(klass(r) != "HIT" for r in ap):
        return "STOP", "DPOSH-MISS", notes + ["applied but not HIT: " + ", ".join(r["id"] for r in ap if klass(r) != "HIT")]
    if any(r["ms"] > PLANT_WALL_MS for r in ap):
        return "STOP", "DPOSH-SLOW", notes + ["over %d ms: %s" % (PLANT_WALL_MS, ", ".join(r["id"] for r in ap if r["ms"] > PLANT_WALL_MS))]
    return "PASS", "", notes


def dstallh_eval(rows, se=no_stderr):
    """Design 6.3 and M1 ruling 4: three plants on arm C (leak -> L, barrier -> B, holdg -> M), 10 seeds each. Per plant applied in at least 95% of its used runs; every applied
    run is class STALL (exit 4) of the expected subtype with the property site TRUE (a plant must not be called a lost wakeup) and ends within PLANT_WALL_MS."""
    notes_all = []
    allrs = sel(rows, "DSH")
    expect = [(first + i, PLANT_KS[i % len(PLANT_KS)]) for first, _ in STALL_PLANTS.values() for i in range(10)]
    st, used, notes = planted_common(allrs, expect, se, "DSTALLH", None)
    if st != "OK":
        return st, "DSTALLH" if st == "MISSING" else used, notes
    notes_all += notes
    for arm, (first, sub) in STALL_PLANTS.items():
        mine = [r for r in used if r["arm"] == arm]
        ap = [r for r in mine if r["applied"] == 1]
        notes_all.append("D-stall-H %s (expect %s): applied %d of %d used runs; STALL %d; subtypes %s; max wall %d ms" % (
            arm, sub, len(ap), len(mine), sum(1 for r in ap if klass(r) == "STALL"), sorted({base_subtype(r["subtype"]) for r in ap}), max([r["ms"] for r in ap], default=0)))
        if len(ap) * 100 < APPLIED_PCT * len(mine):
            return "STOP", "DSTALLH-NOTAPPLIED", notes_all
        if any(klass(r) == "HIT" or r["prop_false"] == 1 for r in ap):
            return "STOP", "DSTALLH-PROP", notes_all + ["the property site was false in a stall plant: " + ", ".join(r["id"] for r in ap if klass(r) == "HIT" or r["prop_false"] == 1)]
        if any(klass(r) != "STALL" for r in ap):
            return "STOP", "DSTALLH-MISS", notes_all + ["applied but not STALL: " + ", ".join(r["id"] for r in ap if klass(r) != "STALL")]
        if any(base_subtype(r["subtype"]) != sub for r in ap):
            return "STOP", "DSTALLH-SUBTYPE", notes_all + ["wrong subtype: " + ", ".join("%s=%s" % (r["id"], r["subtype"]) for r in ap if base_subtype(r["subtype"]) != sub)]
        if any(r["ms"] > PLANT_WALL_MS for r in ap):
            return "STOP", "DSTALLH-SLOW", notes_all + ["over %d ms: %s" % (PLANT_WALL_MS, ", ".join(r["id"] for r in ap if r["ms"] > PLANT_WALL_MS))]
    return "PASS", "", notes_all


# ---------------------------------------------------------------- G1 (P-u), G2a/G2b (recorded), G2c (the detection gate)
def counts(rs):
    c = {k: 0 for k in CLASSES}
    for r in rs:
        c[klass(r)] += 1
    c["rated"] = sum(c[k] for k in RATED)
    c["stallq"] = sum(1 for r in rs if klass(r) == "STALL" and base_subtype(r["subtype"]) == "Q")
    c["stall_nonq"] = c["STALL"] - c["stallq"]
    return c


def g1_eval(rows, se=no_stderr):
    """P-u, 200 seeds, SDK build. Order: HARNESS-FAIL, DEADLOCK-REPORT, CRASH each stop (design 7.3); no rated run; a STALL of subtype L, B or M; fewer than 3 hits
    (G1-PU-LOW-STALLQ when its STALL-Q count is at least 3, else G1-PU-LOW); else PASS, labelled +WITH-NONCOMPLETE when a run was a TIMEOUT or a Hermit failure."""
    rs = sel(rows, "G1", arm="p")
    if len({r["seed"] for r in rs}) < len(G1_SEEDS):
        return "MISSING", "G1", ["G1 rows present: %d of %d" % (len(rs), len(G1_SEEDS))]
    c = counts(rs)
    notes = ["G1 P-u: runs %d rated %d hits %s | STALL-Q %d STALL-other %d | timeout %d hermit-fail %d crash %d harness %d deadlock %d" % (
        len(rs), c["rated"], rate_text(c["HIT"], c["rated"]), c["stallq"], c["stall_nonq"], c["TIMEOUT"], c["HERMIT-FAIL"], c["CRASH"], c["HARNESS-FAIL"], c["DEADLOCK-REPORT"])]
    if c["HARNESS-FAIL"]:
        return "STOP", "G1-HARNESS", notes
    if c["DEADLOCK-REPORT"]:
        return "STOP", "G1-DEADLOCK", notes
    if c["CRASH"]:
        return "STOP", "G1-CRASH", notes
    if c["rated"] == 0:
        return "STOP", "G1-NO-RATED", notes
    if c["stall_nonq"]:
        return "STOP", "G1-STALL-NONQ", notes
    if c["HIT"] < MIN_P_HITS:
        return "STOP", ("G1-PU-LOW-STALLQ" if c["stallq"] >= STALLQ_LOW else "G1-PU-LOW"), notes
    return "PASS", ("+WITH-NONCOMPLETE" if c["TIMEOUT"] or c["HERMIT-FAIL"] else ""), notes


def g2c_eval(rows, se=no_stderr):
    rs = sel(rows, "G2C", arm="pn")
    if len({r["seed"] for r in rs}) < len(G1_SEEDS):
        return "MISSING", "G2C", ["G2C rows present: %d of %d" % (len(rs), len(G1_SEEDS))]
    c = counts(rs)
    notes = ["G2C violation-only P-u: runs %d rated %d hits %s | timeout %d hermit-fail %d crash %d harness %d deadlock %d" % (
        len(rs), c["rated"], rate_text(c["HIT"], c["rated"]), c["TIMEOUT"], c["HERMIT-FAIL"], c["CRASH"], c["HARNESS-FAIL"], c["DEADLOCK-REPORT"])]
    g1 = counts(sel(rows, "G1", arm="p"))
    if g1["rated"] and c["rated"]:
        notes.append("Fisher two-sided p, violation-only %d/%d against SDK %d/%d: %.4g (information only)" % (c["HIT"], c["rated"], g1["HIT"], g1["rated"],
                                                                                              fisher_two_sided(c["HIT"], c["rated"], g1["HIT"], g1["rated"])))
    if c["HARNESS-FAIL"] or c["DEADLOCK-REPORT"] or c["CRASH"]:
        return "STOP", "G2C-HARNESS", notes
    if c["rated"] == 0:
        return "STOP", "G2C-NO-RATED", notes
    if c["HIT"] < MIN_P_HITS:
        return "STOP", "G2C-PU-LOW", notes
    return "PASS", "", notes


def g2a_lines(rows):
    """G2a (equal fingerprints, expected 0 of 10) and G2b (syscalls SDK minus violation-only; the design's formula 33 is a hypothesis, and Go's G2b did not hold: the SDK changes
    the schedule by more than its own report writes). Recorded, never a gate: nothing in the chain reads these lines."""
    out, equal, diffs = [], 0, []
    done = 0
    for s in G2A_SEEDS:
        a = [r for r in sel(rows, "G2A", arm="u") if r["seed"] == s]
        b = [r for r in sel(rows, "G2A", arm="un") if r["seed"] == s]
        if not a or not b:
            continue
        done += 1
        a, b = a[0], b[0]
        same = a["fp"] == b["fp"] and a["fp"] not in ("NOSUMMARY", "")
        equal += 1 if same else 0
        both_clean = klass(a) == "NOHIT" and klass(b) == "NOHIT" and a["syscalls"] >= 0 and b["syscalls"] >= 0
        d = a["syscalls"] - b["syscalls"] if both_clean else None
        diffs.append(d)
        out.append("G2 seed %d: fingerprints %s | classes %s/%s | syscalls %d/%d | difference %s" % (s, "EQUAL" if same else "differ", klass(a), klass(b), a["syscalls"], b["syscalls"], "n/a (a hit or a failed run)" if d is None else d))
    out.append("G2a: equal fingerprints in %d of %d seeds compared (design expectation 0 of 10)" % (equal, done))
    clean = [d for d in diffs if d is not None]
    out.append("G2b: differences on clean pairs %s (design formula %d [+1 if the first high descriptor is refused] is a hypothesis; Go's G2b did not hold; recorded, not a gate)" % (sorted(set(clean)) or "none", SYSCALL_DIFF_FORMULA))
    return out, done


def step_eval(step, rows, rnd, g0s, tp_text, se, part="A"):
    """-> (kind, code_or_labels, notes). The one dispatch used by the driver's gate reads and by the verdict. A step that does not belong to the part is MISSING."""
    if step not in PART_STEPS[part]:
        return "MISSING", step, []
    if step == "tp":
        return tp_eval(parse_tp(tp_text), part)
    if step == "g0":
        k, c, n, _ = g0_eval(rows, rnd, g0s, se, part)
        return k, c, n
    if step == "dposh":
        return dposh_eval(rows, se)
    if step == "dstallh":
        return dstallh_eval(rows, se)
    if step == "g1":
        return g1_eval(rows, se)
    if step == "g2c":
        return g2c_eval(rows, se)
    return "MISSING", step, []


# ---------------------------------------------------------------- the state dump of a hit (reported, never gating)
def parse_dump(line):
    m = re.match(r"^condlock: (HIT|STALL) (.*)$", line)
    if not m:
        return None
    d = {"what": m.group(1)}
    for k, v in re.findall(r"([A-Za-z0-9_>]+)=(\S+)", m.group(2)):
        d.setdefault(k, v)
    return d


def pair(s):
    m = re.match(r"^(\d+),(\d+)$", s or "")
    return (int(m.group(1)), int(m.group(2))) if m else None


def dump_pattern(line):
    """DP-SHAPE (X2 6.6): layout U, lock free, at least one waiter, and for one group g (h the other): g_signals[g] = 2, g_size[g] = 0, g_refs[g] = 0, g_refs[h] = 2 x waiting.
    DP-OTHER: a parsable layout-U dump without the shape. DP-NA-LAYOUT: layout C or none. DP-UNPARSABLE: anything else."""
    d = parse_dump(line)
    if d is None:
        return "DP-UNPARSABLE"
    if d.get("cond") in ("C", "none"):
        return "DP-NA-LAYOUT"
    if d.get("cond") != "U":
        return "DP-UNPARSABLE"
    try:
        waiting, locked, g1 = int(d["waiting"]), int(d["locked"]), int(d["g1"])
    except (KeyError, ValueError):
        return "DP-UNPARSABLE"
    refs, size, sig = pair(d.get("g_refs")), pair(d.get("g_size")), pair(d.get("g_signals"))
    if refs is None or size is None or sig is None or g1 not in (0, 1):
        return "DP-UNPARSABLE"
    if locked == 0 and waiting >= 1:
        for g in (0, 1):
            if sig[g] == 2 and size[g] == 0 and refs[g] == 0 and refs[1 - g] == 2 * waiting:
                return "DP-SHAPE"
    return "DP-OTHER"


def parse_dumps(text):
    out = []
    for line in text.splitlines():
        f = line.split("\t", 3)
        if len(f) == 4:
            out.append(tuple(f))
    return out


# ---------------------------------------------------------------- steps.tsv facts, the chain and the verdict
def phase_facts(steps_text):
    """-> gate failure, binary changed, pause recorded, the first stop reason (for INCONCLUSIVE), the gate reads."""
    gate, bin_changed, pause, reason = None, False, False, None
    for line in steps_text.splitlines():
        n = line.split("\t", 1)[1] if "\t" in line else line
        m = re.match(r"GATE-FAIL (\S+)", n)
        if m and gate is None:
            gate = m.group(1)
        if n.startswith("BIN-CHANGED"):
            bin_changed = True
        if n.startswith("PAUSE-BUDGET"):
            pause = True
        for tag, key in (("BUDGET:", "BUDGET"), ("HALTED", "HALTED"), ("SIGNAL", "SIGNAL"), ("IDLE-EXHAUSTED", "IDLE-EXHAUSTED"), ("REFUSED", "REFUSED"),
                         ("SIZE-CAP", "SIZE-CAP"), ("STOPPED", "STOPPED"), ("TIMEOUT-HALT", "TIMEOUT-HALT")):
            if n.startswith(tag) and reason is None:
                reason = key
    return gate, bin_changed, pause, reason


def chain_code(gate, bin_changed, res, pause, g2a_done, reason=None, part="A"):
    """First rule that holds. res maps each step of the part to (kind, code-or-labels). Always returns a GLG-<part>- code (GLG-A-... or GLG-P-...).
    PASS needs every step of the part PASS and no pause; in part A it also needs the 20 G2A runs made (g2a_done); the labels of the passing steps are appended.
    The pause is recorded after G0, so a pause counts only when tp and g0 passed."""
    pre = "GLG-%s-" % part
    steps = PART_STEPS[part]
    if gate:
        return pre + "NOT-STARTED/%s" % gate
    if bin_changed:
        return pre + "INVALID/BIN-CHANGED"
    for s in steps:
        k, c = res.get(s, ("MISSING", s))
        if k == "STOP":
            return pre + "STOP/%s" % c
    early = all(res.get(s, ("MISSING", ""))[0] == "PASS" for s in steps[:2])
    if pause and early:
        return pre + "PAUSE/BUDGET"
    order = list(steps) + (["g2a"] if part == "A" else [])
    for s in order:
        k = "PASS" if s == "g2a" and g2a_done else res.get(s, ("MISSING", s))[0] if s != "g2a" else "MISSING"
        if k != "PASS":
            return pre + "INCONCLUSIVE/%s-%s" % (s, reason or "NOT-RUN")
    return pre + "PASS" + "".join(res[s][1] for s in steps if res[s][1])


def line_rate(label, rs):
    c = counts(rs)
    return "%s: runs=%d rated=%d hit runs=%s | timeout=%d hermit-fail=%d crash=%d harness=%d deadlock=%d stall=%d (Q %d)" % (
        label, len(rs), c["rated"], rate_text(c["HIT"], c["rated"]), c["TIMEOUT"], c["HERMIT-FAIL"], c["CRASH"], c["HARNESS-FAIL"], c["DEADLOCK-REPORT"], c["STALL"], c["stallq"])


def fmt_ev(a, ev):
    mt = "n/a" if ev["mean_ticks"] is None else "%.2f" % ev["mean_ticks"]
    return "  %s: runs=%d healthy=%d | G0w maxstag=%d (limit %d) mean_ticks=%s%s | contended_signal true in %d runs, wait_spanned_two_signals true in %d runs | blocked1_sum=%d spanned_sum=%d | hits=%d stalls=%d" % (
        "P-u" if a == "p" else "A-" + a, ev["runs"], ev["healthy"], ev["maxstag"], STAG_MAX, mt, " SHORT-TICKS" if short_ticks(ev) else "", ev["ct_runs"], ev["wt_runs"],
        ev["blocked1_sum"], ev["spanned_sum"], ev["hits"], ev["stalls"])


def main_verdict(run_dir):
    rows = parse_rows(read_text(os.path.join(run_dir, "runs.tsv")))
    tp_text = read_text(os.path.join(run_dir, "tp.tsv"))
    steps_text = read_text(os.path.join(run_dir, "steps.tsv"))
    se = stderr_reader(os.path.join(run_dir, "out"))
    args = read_text(os.path.join(run_dir, "out", "args.txt")).split()
    rnd, g0s = (int(args[0]), int(args[1])) if len(args) >= 2 and all(a.isdigit() for a in args[:2]) else (500, 319101)
    part = args[2] if len(args) >= 3 and args[2] in PARTS else "A"
    gate, bin_changed, pause, reason = phase_facts(steps_text)
    print("part     :", part)
    print("gate     :", gate or "none", "| binary changed:", bin_changed, "| pause recorded:", pause, "| first stop reason:", reason or "none")
    print("launch   : RND=%d G0S=%d (launch table row: RND %s)" % (rnd, g0s, LAUNCHES.get(g0s, "n/a")))
    res = {}
    for s in PART_STEPS[part]:
        k, c, n = step_eval(s, rows, rnd, g0s, tp_text, se, part)
        res[s] = (k, c)
        print("step %-8s: %s %s" % (s, k, c))
        for line in n:
            print("    " + line)
    if part == "A" and res["g0"] == ("STOP", "G0T-LOWER-R") and rnd in RND_LADDER:
        nr = next_rnd(rnd)
        print("NEXT-LAUNCH (launch table, spec 6.8): RND=%d G0S=%d" % (nr, [b for b, r in LAUNCHES.items() if r == nr][0]))
    kind, code, _, info = g0_eval(rows, rnd, g0s, se, part)
    eff = info["eff"]
    print("G0 per (arm, seed) [replaced pairs show their spare pair]:")
    for (arm, s), v in sorted(info["per"].items(), key=lambda kv: (kv[0][1], ARMS.index(kv[0][0]))):
        print("  %s seed %d (ran with %d): %s | fingerprints %s" % (arm, s, eff[(arm, s)][1], v, sorted({r["fp"] for r in eff[(arm, s)][0]})))
    for a in PART_ARMS[part]:
        print(fmt_ev(a, arm_evidence(info["runs"][a])))
    ledger = {}
    for r in rows:
        if klass(r) in ("HERMIT-FAIL", "TIMEOUT", "CRASH", "HARNESS-FAIL", "DEADLOCK-REPORT"):
            key = (r["step"], r["arm"], klass(r))
            n = ledger.setdefault(key, [0, 0])
            n[0] += 1
            n[1] += 1 if klass(r) == "HERMIT-FAIL" and skid_run(r, se(r["id"])) else 0
    print("Failure ledger (step, arm, class: runs, of which SKID_OVERSHOOT signature):", {k: tuple(v) for k, v in sorted(ledger.items())} or "none")
    if info.get("medians"):
        mu, mc = info["medians"]
        print("C1 planning figure (not a gate): 2,000 x (A-u + A-c medians) = %.0f s = %.2f box hours at RND %d" % (C1_RUNS_PER_ARM * (mu + mc) / 1000.0, C1_RUNS_PER_ARM * (mu + mc) / 3.6e6, rnd))
    done = 0
    if part == "P":
        print(line_rate("G1 P-u (SDK build)", sel(rows, "G1", arm="p")))
        print(line_rate("G2C P-u (violation-only build)", sel(rows, "G2C", arm="pn")))
    else:
        lines, done = g2a_lines(rows)
        for line in lines:
            print(line)
    tally = {}
    for _id, arm, cls_, line in parse_dumps(read_text(os.path.join(run_dir, "dumps.tsv"))):
        t = tally.setdefault(arm, {})
        lab = dump_pattern(line)
        t[lab] = t.get(lab, 0) + 1
    print("DUMP-TALLY (reported, never gating):", {a: dict(sorted(t.items())) for a, t in sorted(tally.items())} or "no dumps")
    pk = [r for r in rows if r["step"] in ("DPH", "DSH")]
    print("planted steps: rows=%d applied=%d" % (len(pk), sum(1 for r in pk if r["applied"] == 1)))
    print("OUTCOME", chain_code(gate, bin_changed, res, pause, done == len(G2A_SEEDS), reason, part))


# ---------------------------------------------------------------- selftest
def _row(**kw):
    r = {c: 0 for c in COLS}
    r.update({"id": "x", "step": "G0", "arm": "u", "k": 0, "seed": 319101, "rep": 1, "rc": 0, "ms": 1000, "cls": "NOHIT", "subtype": "-", "applied": -1, "libc_ok": 1,
              "rounds": 4000, "signals": 4000, "blocked1": 3000, "maxb": 7, "maxstag": 0, "ticks": 18, "spanned": 1900, "nout": 1, "ct": 2000, "wt": 1900,
              "stdout_sha": "aaaa", "fp": "ffff", "syscalls": 800, "virt_ns": 4000000000, "turns": 9000, "dump": 0})
    r.update(kw)
    return r


SKID_ERR = ("hermit: virtual-time epoch=2026-01-01T00:00:00+00:00\nthread 'main' panicked at detcore/src/lib.rs:348:9:\nHERMIT_SKID_OVERSHOOT rcb_actual=8482180 rcb_target=8482179 skid_margin=25 overshoot=1\n"
            "HERMIT_TASK_PANIC message=HERMIT_SKID_OVERSHOOT\nHERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)\n")


def mk_g0(g0s=319101, ms=None, fail=None, fp=None, arms=("u", "c")):
    """A clean G0 row set: 10 seeds x the arms of the part (default part A: u and c; part P: ("p",)) x 3 reps. ms(arm) gives the wall; fail maps (arm, seed, rep) to a dict of row
    overrides; fp(arm, seed) the fingerprint."""
    rows = []
    for s in seedlist(g0s):
        for a in arms:
            for rep in (1, 2, 3):
                kw = {"id": "G0.%s.s%d.r%d" % (a, s, rep), "step": "G0", "arm": a, "seed": s, "rep": rep, "ms": ms(a) if ms else 1000,
                      "fp": fp(a, s) if fp else "fp%s%d" % (a, s), "stdout_sha": "so%s%d" % (a, s)}
                kw.update((fail or {}).get((a, s, rep), {}))
                rows.append(_row(**kw))
    return rows


def _selftest_stats():
    assert abs(100 * wilson(0, 2000)[1] - 0.1917) < 5e-5, wilson(0, 2000)
    assert abs(100 * zero_bound(2000) - 0.1497) < 5e-5 and abs(100 * zero_bound(1000) - 0.2991) < 5e-4
    assert abs(100 * wilson(3, 200)[0] - 0.511) < 5e-4, wilson(3, 200)
    assert abs(100 * zero_bound(200) - 1.4888) < 5e-3 and abs(100 * wilson(0, 200)[1] - 1.885) < 5e-3, wilson(0, 200)
    for k, want in ((7, 0.170), (8, 0.203)):
        assert abs(100 * wilson(k, 2000)[0] - want) < 5e-4, (k, wilson(k, 2000))
    assert wilson(0, 0) == (None, None) and zero_bound(0) is None
    # Fisher: the classic tea-tasting table (3,1 / 1,3 -> two-sided p = 0.4857) and a brute-force enumeration
    assert abs(fisher_two_sided(3, 4, 1, 4) - 0.4857142857) < 1e-9
    assert abs(fisher_two_sided(0, 10, 10, 10) - 2 * math.comb(10, 0) * math.comb(10, 10) / math.comb(20, 10)) < 1e-12
    assert fisher_two_sided(5, 10, 5, 10) == 1.0 and fisher_two_sided(0, 5, 0, 5) == 1.0
    assert median([3, 1, 2]) == 2 and median([1, 2, 3, 4]) == 2.5 and median([]) is None


def _selftest_classes():
    assert klass(_row()) == "NOHIT" and klass(_row(cls="HIT", rc=3, prop_false=1)) == "HIT" and klass(_row(cls="STALL", rc=4, stall_hit=1, subtype="Q")) == "STALL"
    assert klass(_row(cls="HIT", rc=0, prop_false=1)) == "HARNESS-FAIL" and klass(_row(cls="HIT", rc=3, prop_false=0)) == "HARNESS-FAIL"
    assert klass(_row(cls="STALL", rc=3, stall_hit=1)) == "HARNESS-FAIL" and klass(_row(cls="NOHIT", rc=0, stall_hit=1)) == "HARNESS-FAIL"
    assert klass(_row(libc_ok=0)) == "HARNESS-FAIL" and klass(_row(nout=2)) == "HARNESS-FAIL" and klass(_row(cls="TIMEOUT", rc=124, libc_ok=0)) == "TIMEOUT"
    assert klass(_row(cls="CRASH", rc=139)) == "CRASH" and klass(_row(cls="HERMIT-FAIL", rc=125)) == "HERMIT-FAIL" and klass(_row(cls="DEADLOCK-REPORT", rc=1)) == "DEADLOCK-REPORT"
    assert klass(_row(cls="ERR")) == "HARNESS-FAIL" and klass(_row(cls="HARNESS-FAIL", rc=5)) == "HARNESS-FAIL"
    # the SKID_OVERSHOOT signature: strict
    sk = _row(cls="HERMIT-FAIL", rc=125)
    assert skid_run(sk, SKID_ERR)
    assert not skid_run(_row(cls="HERMIT-FAIL", rc=101), SKID_ERR) and not skid_run(_row(cls="HERMIT-FAIL", rc=125, ), "")
    assert not skid_run(_row(cls="CRASH", rc=125), SKID_ERR) and not skid_run(_row(cls="NOHIT", rc=125), SKID_ERR)
    assert not skid_run(sk, SKID_ERR + "HERMIT_CLEANUP_UNCONFIRMED x\n") and not skid_run(sk, SKID_ERR + "Deadlock detected: x\n")
    assert not skid_run(sk, SKID_ERR + "thread 'x' panicked at detcore/src/other.rs:1:1\n") and not skid_run(sk, SKID_ERR.replace("lib.rs:348", "lib.rs:349"))
    assert not skid_run(sk, SKID_ERR.replace("Exited(101)", "Exited(102)")) and not skid_run(sk, SKID_ERR + "HERMIT_TASK_PANIC message=other\n")
    assert not skid_run(sk, SKID_ERR.replace("rcb_target=8482179 ", "")) and not skid_run(sk, SKID_ERR + "Usage: hermit run\n") and not skid_run(sk, SKID_ERR + "x tool_global.rs:1619 y\n")
    assert skid_run(_row(cls="HERMIT-FAIL", rc=125, prop_false=1, fp="zz"), SKID_ERR)       # the decision never reads a hit or a fingerprint


def _selftest_timing_wall_only():
    assert timing_a([2700] * 30, [2700] * 30, 500) == (None, (2700, 2700)) and timing_a([2701] * 30, [100] * 30, 500)[0] == "G0T-LOWER-R"
    assert timing_a([100] * 30, [2700.5] * 30, 125)[0] == "G0T-LOWER-R" and timing_a([2701] * 30, [100] * 30, 60)[0] == "G0T-LOWER-R"
    assert timing_a([2701] * 30, [100] * 30, 30)[0] == "G0T-C1-BUDGET" and timing_a([30000] * 30, [100] * 30, 30)[0] == "G0T-C1-BUDGET" and timing_a([30001] * 30, [100] * 30, 30)[0] == "G0T-KILL"
    assert timing_a([100] * 30, [30001] * 30, 30)[0] == "G0T-KILL" and timing_a([100] * 30, [2700] * 30, 30)[0] is None
    assert timing_p([6000] * 30)[0] is None and timing_p([6001] * 30)[0] == "G0T-P-SLOW" and timing_p([])[0] is None
    assert [next_rnd(x) for x in RND_LADDER] == [250, 125, 60, 30, 30] and all(LAUNCHES[b] == r for b, r in zip((319101, 319511, 319521, 319531, 319541), RND_LADDER))
    # What this checks (and no more): through g0_eval, A-u row sets that differ only in class, hit and stall flags (every run of a seed is a NOHIT, a HIT or a STALL, all legal runs)
    # and have the SAME measured wall times give the same G0t decision. A hit run ends early under the watchdog, so its wall time enters the median as measured: the decision is a
    # function of the wall times of the effective runs and RND, and the code never reads a class or a hit count to make it. (timing_a itself receives only milliseconds.)
    import random
    rnd_ = random.Random(7)
    g0s = 319101
    kinds = (dict(), dict(cls="HIT", rc=3, prop_false=1, maxstag=10), dict(cls="STALL", rc=4, stall_hit=1, subtype="Q", maxstag=10))
    for rnd in RND_LADDER:
        for _ in range(12):
            ms_by_seed = {s: rnd_.choice([500, 2600, 2800, 5000]) for s in seedlist(g0s)}
            plain, mixed = [], []
            for s in seedlist(g0s):
                kind = rnd_.choice(kinds)
                for a in PART_ARMS["A"]:
                    for rep in (1, 2, 3):
                        base_kw = {"id": "G0.%s.s%d.r%d" % (a, s, rep), "step": "G0", "arm": a, "seed": s, "rep": rep, "fp": "fp%s%d" % (a, s), "stdout_sha": "so%s%d" % (a, s),
                                   "ms": ms_by_seed[s] if a == "u" else 1000}
                        plain.append(_row(**base_kw))
                        mixed.append(_row(**dict(base_kw, **(kind if a == "u" else {}))))
            rp, rm = g0_eval(plain, rnd, g0s), g0_eval(mixed, rnd, g0s)
            tc = lambda r: r[1] if r[1].startswith("G0T-") else None
            assert tc(rp) == tc(rm), (rnd, ms_by_seed, rp[1], rm[1])
            med = median([ms_by_seed[s] for s in seedlist(g0s) for _r in range(3)])
            assert (tc(rp) is not None) == (med > A_MEDIAN_LIMIT_MS), (rnd, med, rp[1])
    assert len(SPARE_PAIRS) == 5 and len({s for p in SPARE_PAIRS.values() for s in p}) == 10 and min(s for p in SPARE_PAIRS.values() for s in p) == 319561


def _selftest_g0():
    g0s = 319101
    ok = mk_g0()
    k, c, n, info = g0_eval(ok, 500, g0s)
    assert (k, c) == ("PASS", ""), (k, c, n)
    # labels: a short-tick arm is labelled, not stopped
    k, c, _, _ = g0_eval(mk_g0(fail={(a, s, r): {"ticks": 1} for a in ("c",) for s in seedlist(g0s) for r in (1, 2, 3)}), 500, g0s)
    assert (k, c) == ("PASS", "+SHORT-TICKS-C"), (k, c)
    # every code of G0 is reached
    def code(rows, rnd=500, se=no_stderr):
        r = g0_eval(rows, rnd, g0s, se)
        return r[0], r[1]
    nd = {("u", 319103, 2): {"fp": "other"}}
    assert code(mk_g0(fail=nd)) == ("STOP", "G0-NONDET") and code(mk_g0(fail={("c", 319103, 3): {"stdout_sha": "x"}})) == ("STOP", "G0-NONDET")
    assert code(mk_g0(fail={("u", 319103, 3): {"fp": "x", "stdout_sha": "y"}})) == ("STOP", "G0-NONDET")
    allbad = {(a, 319101, r): {"cls": "HERMIT-FAIL", "rc": 101} for a in PART_ARMS["A"] for r in (1, 2, 3)}
    assert code(mk_g0(fail=allbad)) == ("STOP", "G0-ALLBAD")
    assert code(mk_g0(fail={("u", 319104, 1): {"cls": "HERMIT-FAIL", "rc": 101}})) == ("STOP", "G0-INCOMPLETE")
    for c_ in ("CRASH", "HARNESS-FAIL", "DEADLOCK-REPORT"):
        assert code(mk_g0(fail={("c", 319104, 1): {"cls": c_, "rc": 5}})) == ("STOP", "G0-INCOMPLETE"), c_
    assert code(mk_g0(fail={("c", 319104, 1): {"cls": "HIT", "rc": 3, "prop_false": 1}})) == ("STOP", "G0-CONTROL-HIT")
    assert code(mk_g0(fail={("c", 319104, 1): {"cls": "STALL", "rc": 4, "stall_hit": 1, "subtype": "Q"}})) == ("STOP", "G0-CONTROL-STALL")
    assert code(mk_g0(fail={("u", 319104, 1): {"cls": "HIT", "rc": 3, "prop_false": 1, "fp": "h1"}, ("u", 319104, 2): {"cls": "HIT", "rc": 3, "prop_false": 1, "fp": "h1"},
                            ("u", 319104, 3): {"cls": "HIT", "rc": 3, "prop_false": 1, "fp": "h1"}})) == ("PASS", "")     # an A-u hit is a legal run
    assert code(ok[:-1]) == ("MISSING", "G0")
    slow = mk_g0(ms=lambda a: 2800 if a == "u" else 1000)
    assert code(slow, 500) == ("STOP", "G0T-LOWER-R") and code(slow, 60) == ("STOP", "G0T-LOWER-R") and code(slow, 30) == ("STOP", "G0T-C1-BUDGET")
    assert code(mk_g0(ms=lambda a: 30001 if a == "c" else 1000), 30) == ("STOP", "G0T-KILL")
    to = mk_g0(fail={("u", 319104, 1): {"cls": "TIMEOUT", "rc": 124, "ms": 120000}})
    assert code(to) == ("STOP", "G0-INCOMPLETE")
    assert code(mk_g0(fail={("c", 319105, 2): {"maxstag": 4}})) == ("STOP", "G0W-STAG")           # one healthy run with 4 stagnant ticks is enough
    assert code(mk_g0(fail={("c", 319105, 2): {"maxstag": 3}})) == ("PASS", "")                    # 3 is the limit (a margin of 10/3 against Q = 10)
    assert code(mk_g0(fail={("u", 319105, r): {"maxstag": 4} for r in (1, 2, 3)})) == ("STOP", "G0W-STAG")
    assert g0_eval(mk_g0(arms=("u", "c", "p")), 500, g0s)[0:2] == ("PASS", "")                     # a stray P row in part A is ignored: no P rule is read in part A
    assert code(mk_g0(fail={("u", 319105, r): {"maxstag": 12, "cls": "HIT", "rc": 3, "prop_false": 1} for r in (1, 2, 3)}))[0] == "PASS"      # a hit run is not a healthy run
    assert code(mk_g0(fp=lambda a, s: "same" if a == "u" else "fp%s%d" % (a, s))) == ("STOP", "G0D-NODIV")
    assert code(mk_g0(fp=lambda a, s: ("two" if s % 2 else "one") if a == "u" else "fp%s%d" % (a, s))) == ("STOP", "G0D-NODIV")
    assert code(mk_g0(fp=lambda a, s: ("t%d" % (s % 3)) if a == "u" else "fp%s%d" % (a, s))) == ("PASS", "")
    assert code(mk_g0(fail={("u", s, r): {"ct": 0} for s in seedlist(g0s) for r in (1, 2, 3)})) == ("STOP", "G0C-NO-CONTENDED")
    assert code(mk_g0(fail={("u", s, r): {"wt": -1} for s in seedlist(g0s) for r in (1, 2, 3)})) == ("STOP", "G0C-NO-SPAN")
    assert code(mk_g0(fail={("c", s, r): {"ct": 0, "wt": 0} for s in seedlist(g0s) for r in (1, 2, 3)})) == ("PASS", "")                      # A-c coverage is reported, not gated
    assert code(mk_g0(fail={("u", 319102, 1): {"fp": "NOSUMMARY"}, ("u", 319102, 2): {"fp": "NOSUMMARY"}, ("u", 319102, 3): {"fp": "NOSUMMARY"}})) == ("STOP", "G0-INCOMPLETE")
    # the first-seed check
    assert g0_first(mk_g0(fail=allbad), g0s) == "ALLBAD" and g0_first(ok, g0s) == "OK" and g0_first([r for r in ok if r["seed"] != g0s], g0s) == "OK"


def _selftest_g0_p():
    """PART P: the P-u triples, G0t for P at the frozen RND, G0w for P. No A rule is read."""
    g0s = 319101
    P = ("p",)
    ok = mk_g0(arms=P)
    assert len(ok) == 30 and g0_eval(ok, 500, g0s, no_stderr, "P")[0:2] == ("PASS", "")
    code = lambda rows, rnd=500: g0_eval(rows, rnd, g0s, no_stderr, "P")[0:2]
    assert code(mk_g0(arms=P, fail={("p", 319103, 2): {"fp": "other"}})) == ("STOP", "G0-NONDET") and code(mk_g0(arms=P, fail={("p", 319103, 3): {"stdout_sha": "x"}})) == ("STOP", "G0-NONDET")
    allbad = {("p", 319101, r): {"cls": "HERMIT-FAIL", "rc": 101} for r in (1, 2, 3)}
    assert code(mk_g0(arms=P, fail=allbad)) == ("STOP", "G0-ALLBAD") and g0_first(mk_g0(arms=P, fail=allbad), g0s, "P") == "ALLBAD" and g0_first(mk_g0(arms=P, fail=allbad), g0s, "A") == "OK"
    assert code(mk_g0(arms=P, fail={("p", 319104, 1): {"cls": "HERMIT-FAIL", "rc": 101}})) == ("STOP", "G0-INCOMPLETE")
    assert code(ok[:-1]) == ("MISSING", "G0")
    assert code(mk_g0(arms=P, ms=lambda a: 6000)) == ("PASS", "") and code(mk_g0(arms=P, ms=lambda a: 6001)) == ("STOP", "G0T-P-SLOW")
    assert code(mk_g0(arms=P, ms=lambda a: 6001), 30) == ("STOP", "G0T-P-SLOW")                      # at any RND: P's RND is never lowered on its own, the stop needs a human decision
    assert code(mk_g0(arms=P, fail={("p", 319104, 1): {"cls": "TIMEOUT", "rc": 124, "ms": 120000}})) == ("STOP", "G0-INCOMPLETE")
    assert code(mk_g0(arms=P, fail={("p", 319105, 2): {"maxstag": 4}})) == ("STOP", "G0W-STAG") and code(mk_g0(arms=P, fail={("p", 319105, 2): {"maxstag": 3}})) == ("PASS", "")
    assert code(mk_g0(arms=P, fail={("p", s, r): {"ticks": 1} for s in seedlist(g0s) for r in (1, 2, 3)})) == ("PASS", "+SHORT-TICKS-P")
    assert code(mk_g0(arms=P, fail={("p", s, r): {"ct": 0, "wt": 0} for s in seedlist(g0s) for r in (1, 2, 3)})) == ("PASS", "")      # coverage is not gated for P-u
    assert code(mk_g0(arms=P, fp=lambda a, s: "same")) == ("PASS", "")                                    # G0d is an A-u rule: not read in part P
    assert code(mk_g0(arms=("u", "c", "p"), ms=lambda a: 9000 if a != "p" else 1000)) == ("PASS", "")      # A rows in part P are ignored: no A timing, control or diversity rule is read
    # SKID replacement in part P uses the same spare pairs, with arm p
    sp = spares_for(g0s)
    skid = {("p", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125, "libc_ok": 0, "ms": 300}}
    base = mk_g0(arms=P, fail=skid)
    se = lambda rid: SKID_ERR if any(r["id"] == rid and r["rc"] == 125 for r in base) else ""
    assert skid_next(base, "p", 319102, g0s, se, "P") == "RUN %d" % sp[0] and skid_next(base, "u", 319102, g0s, se, "P") == "DONE"
    spare = [_row(id="G0R.p.s%d.r%d" % (sp[0], r), step="G0R", arm="p", seed=sp[0], rep=r, fp="spare", stdout_sha="sp") for r in (1, 2, 3)]
    assert g0_eval(base + spare, 500, g0s, se, "P")[0:2] == ("PASS", "")
    # the projected-time check and the TP recheck of part P
    five = mk_g0(arms=P, ms=lambda a: 5000)
    assert projected_pause(five, g0s, 1000, 1000 + 2600, no_stderr, "P")[0] == "OK" and projected_pause(five, g0s, 1000, 1000 + 2599, no_stderr, "P")[0] == "PAUSE"      # need = 2600 s exactly
    need = projected_pause(five, g0s, 0, 99999, no_stderr, "P")[1]
    assert abs(need - (400 * 5.0 + 600)) < 1e-9, need
    assert projected_pause([], g0s, 0, 99999, no_stderr, "P")[0] == "PAUSE"
    good = [dict(id="tp-%s-%s" % k, case=k[0], arm=k[1], rc=0, ms=160, virt_ns=60011973185, turns=34, ok=1) for k in TP_NEED_PART["P"]]
    assert tp_eval(good, "P")[0] == "PASS" and tp_eval(good, "A")[0] == "MISSING" and tp_eval(good[:1], "P")[0] == "MISSING"


def _selftest_skid():
    g0s = 319101
    sp = spares_for(g0s)
    skid = lambda a, s, rep=1: {(a, s, rep): {"cls": "HERMIT-FAIL", "rc": 125, "libc_ok": 0, "ms": 300}}
    se_for = lambda rows_: (lambda rid: SKID_ERR if any(r["id"] == rid and r["rc"] == 125 for r in rows_) else "")
    base = mk_g0(fail=skid("u", 319102))
    se = se_for(base)
    assert skid_next(base, "u", 319102, g0s, se) == "RUN %d" % sp[0] and skid_next(base, "u", 319103, g0s, se) == "DONE" and skid_next(base, "c", 319102, g0s, se) == "DONE"
    assert g0_eval(base, 500, g0s, se)[0] == "MISSING"                                  # the spare pair is not made yet
    spare = [_row(id="G0R.u.s%d.r%d" % (sp[0], r), step="G0R", arm="u", seed=sp[0], rep=r, fp="spare", stdout_sha="sp") for r in (1, 2, 3)]
    done = base + spare
    k, c, notes, info = g0_eval(done, 500, g0s, se)
    assert (k, c) == ("PASS", ""), (k, c, notes)
    assert skid_next(done, "u", 319102, g0s, se) == "DONE" and any("replacement 1 of 2" in n for n in notes)
    assert len(info["runs"]["u"]) == 30 and all(r["id"] != "G0.u.s319102.r1" for r in info["runs"]["u"])
    # the excluded pair is out of the medians: a slow failed run does not move the median
    slowfail = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125, "ms": 900000}})
    sf = slowfail + spare
    assert g0_eval(sf, 500, g0s, se_for(slowfail))[0:2] == ("PASS", "")
    # a skid plus a disagreement in the replaced pair's completed reps is G0-NONDET
    dis = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125}, ("u", 319102, 2): {"fp": "other"}})
    assert g0_eval(dis + spare, 500, g0s, se_for(dis))[0:2] == ("STOP", "G0-NONDET")
    dis2 = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125}, ("u", 319102, 3): {"stdout_sha": "other"}})
    assert g0_eval(dis2 + spare, 500, g0s, se_for(dis2))[0:2] == ("STOP", "G0-NONDET")
    one = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125}, ("u", 319102, 2): {"cls": "HERMIT-FAIL", "rc": 125}})
    assert g0_eval(one + spare, 500, g0s, se_for(one))[0:2] == ("PASS", "")       # two skid reps and one completed run: nothing to compare, one replacement
    # the cap: a third replacement is G0-INCOMPLETE; two are fine
    two = mk_g0(fail={**skid("u", 319102), **skid("c", 319103)})
    sp2 = [_row(id="G0R.c.s%d.r%d" % (sp[1], r), step="G0R", arm="c", seed=sp[1], rep=r, fp="spare2", stdout_sha="sp2") for r in (1, 2, 3)]
    assert g0_eval(two + spare + sp2, 500, g0s, se_for(two))[0:2] == ("PASS", "")
    three = mk_g0(fail={**skid("u", 319102), **skid("c", 319103), **skid("u", 319104)})
    k, c, notes, _ = g0_eval(three + spare + sp2, 500, g0s, se_for(three))
    assert (k, c) == ("STOP", "G0-INCOMPLETE") and any("not replaced" in n for n in notes), (k, c, notes)
    # a spare that also fails uses the next spare of the launch
    spf = [_row(id="G0R.u.s%d.r1" % sp[0], step="G0R", arm="u", seed=sp[0], rep=1, cls="HERMIT-FAIL", rc=125)] + [
        _row(id="G0R.u.s%d.r%d" % (sp[0], r), step="G0R", arm="u", seed=sp[0], rep=r, fp="spare", stdout_sha="sp") for r in (2, 3)]
    base2 = base + spf
    se2 = se_for(base2)
    assert skid_next(base2, "u", 319102, g0s, se2) == "RUN %d" % sp[1]
    sp1b = [_row(id="G0R.u.s%d.r%d" % (sp[1], r), step="G0R", arm="u", seed=sp[1], rep=r, fp="spare1b", stdout_sha="sp1b") for r in (1, 2, 3)]
    assert g0_eval(base2 + sp1b, 500, g0s, se2)[0:2] == ("PASS", "")
    # any other defect is not replaced: another Hermit failure, a timeout, a crash
    for kw in ({"cls": "HERMIT-FAIL", "rc": 101}, {"cls": "TIMEOUT", "rc": 124}, {"cls": "CRASH", "rc": 139}):
        other = mk_g0(fail={("u", 319102, 1): kw})
        assert skid_next(other, "u", 319102, g0s, se_for(other)) == "DONE"
    mixed = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125}, ("u", 319102, 2): {"cls": "CRASH", "rc": 139}})
    assert skid_next(mixed, "u", 319102, g0s, se_for(mixed)) == "DONE"
    # without the stderr text there is no signature
    assert skid_next(base, "u", 319102, g0s, no_stderr) == "DONE"
    # decision is hit-blind: a skid run that also carries a hit fact is still a skid run
    hb = mk_g0(fail={("u", 319102, 1): {"cls": "HERMIT-FAIL", "rc": 125, "prop_false": 1, "fp": "weird"}})
    assert skid_next(hb, "u", 319102, g0s, se_for(hb)) == "RUN %d" % sp[0]


def _selftest_planted():
    def dp(over=None, ms=800):
        rows = []
        for s in DPOS_SEEDS:
            k = PLANT_KS[(s - DPOS_SEEDS[0]) % 5]
            kw = {"id": "DPH.s%d" % s, "step": "DPH", "arm": "d", "k": k, "seed": s, "cls": "HIT", "rc": 3, "prop_false": 1, "applied": 1, "ms": ms, "subtype": "Q+attributed"}
            kw.update((over or {}).get(s, {}))
            rows.append(_row(**kw))
        return rows
    assert dposh_eval(dp())[0:2] == ("PASS", "")
    assert dposh_eval(dp()[:-1])[0] == "MISSING"
    assert dposh_eval(dp({319601: {"applied": 0, "cls": "NOHIT", "rc": 0, "prop_false": 0}}))[0:2] == ("PASS", "")                      # 29 of 30 applied = 96.7%
    assert dposh_eval(dp({319601: {"applied": 0, "cls": "NOHIT", "rc": 0, "prop_false": 0}, 319602: {"applied": 0, "cls": "NOHIT", "rc": 0, "prop_false": 0}}))[0:2] == ("STOP", "DPOSH-NOTAPPLIED")
    assert dposh_eval(dp({319603: {"cls": "NOHIT", "rc": 0, "prop_false": 0}}))[0:2] == ("STOP", "DPOSH-MISS")
    assert dposh_eval(dp({319603: {"ms": 10001}}))[0:2] == ("STOP", "DPOSH-SLOW") and dposh_eval(dp({319603: {"ms": 10000}}))[0:2] == ("PASS", "")
    assert dposh_eval(dp({319603: {"cls": "TIMEOUT", "rc": 124, "applied": -1}}))[0:2] == ("STOP", "DPOSH-BAD")
    assert dposh_eval(dp({319603: {"cls": "CRASH", "rc": 139}}))[0:2] == ("STOP", "DPOSH-BAD") and dposh_eval(dp({319603: {"k": 99}}))[0:2] == ("STOP", "DPOSH-BAD")
    assert dposh_eval(dp({319603: {"applied": 0, "cls": "HIT", "rc": 3, "prop_false": 1}}))[0:2] == ("STOP", "DPOSH-BAD")
    assert dposh_eval(dp({319603: {"applied": -1}}))[0:2] == ("STOP", "DPOSH-BAD")
    skidrow = {319605: {"cls": "HERMIT-FAIL", "rc": 125, "applied": -1}}
    rows = dp(skidrow)
    se = lambda rid: SKID_ERR if rid == "DPH.s319605" else ""
    assert dposh_eval(rows, se)[0:2] == ("PASS", "") and dposh_eval(rows)[0:2] == ("STOP", "DPOSH-BAD")       # excluded only with the signature
    rows3 = dp({s: {"cls": "HERMIT-FAIL", "rc": 125, "applied": -1} for s in (319605, 319606, 319607)})
    assert dposh_eval(rows3, lambda rid: SKID_ERR)[0:2] == ("STOP", "DPOSH-INCOMPLETE")

    def ds(over=None, ms=900):
        rows = []
        for arm, (first, sub) in STALL_PLANTS.items():
            for i in range(10):
                s = first + i
                kw = {"id": "DSH.%s.s%d" % (arm, s), "step": "DSH", "arm": arm, "k": PLANT_KS[i % 5], "seed": s, "cls": "STALL", "rc": 4, "stall_hit": 1, "applied": 1, "ms": ms, "subtype": sub}
                kw.update((over or {}).get(s, {}))
                rows.append(_row(**kw))
        return rows
    assert dstallh_eval(ds())[0:2] == ("PASS", "") and dstallh_eval(ds()[:-1])[0] == "MISSING"
    assert dstallh_eval(ds({319645: {"cls": "HIT", "rc": 3, "prop_false": 1, "stall_hit": 0}}))[0:2] == ("STOP", "DSTALLH-PROP")
    assert dstallh_eval(ds({319645: {"cls": "NOHIT", "rc": 0, "stall_hit": 0, "applied": 1}}))[0:2] == ("STOP", "DSTALLH-MISS")
    assert dstallh_eval(ds({319655: {"subtype": "L"}}))[0:2] == ("STOP", "DSTALLH-SUBTYPE") and dstallh_eval(ds({319665: {"subtype": "Q"}}))[0:2] == ("STOP", "DSTALLH-SUBTYPE")
    assert dstallh_eval(ds({319665: {"applied": 0, "cls": "NOHIT", "rc": 0, "stall_hit": 0}}))[0:2] == ("STOP", "DSTALLH-NOTAPPLIED")      # 9 of 10 is 90%: a plant needs all 10
    assert dstallh_eval(ds({319665: {"ms": 10001}}))[0:2] == ("STOP", "DSTALLH-SLOW") and dstallh_eval(ds({319642: {"cls": "CRASH", "rc": 2}}))[0:2] == ("STOP", "DSTALLH-BAD")
    srow = ds({319665: {"cls": "HERMIT-FAIL", "rc": 125, "applied": -1}})
    assert dstallh_eval(srow, lambda rid: SKID_ERR if rid == "DSH.h.s319665" else "")[0:2] == ("PASS", "")


def _selftest_g1():
    def g1(hits=3, stallq=0, over=None):
        over = over or {}
        rows = []
        for i, s in enumerate(G1_SEEDS):
            kw = {"id": "G1.p.s%d" % s, "step": "G1", "arm": "p", "seed": s}
            if i < hits:
                kw.update({"cls": "HIT", "rc": 3, "prop_false": 1})
            elif i < hits + stallq:
                kw.update({"cls": "STALL", "rc": 4, "stall_hit": 1, "subtype": "Q"})
            kw.update(over.get(s, {}))
            rows.append(_row(**kw))
        return rows
    assert g1_eval(g1(3))[0:2] == ("PASS", "") and g1_eval(g1(200))[0:2] == ("PASS", "") and g1_eval(g1(2))[0:2] == ("STOP", "G1-PU-LOW") and g1_eval(g1(0))[0:2] == ("STOP", "G1-PU-LOW")
    assert g1_eval(g1(2, stallq=3))[0:2] == ("STOP", "G1-PU-LOW-STALLQ") and g1_eval(g1(2, stallq=2))[0:2] == ("STOP", "G1-PU-LOW") and g1_eval(g1(3, stallq=5))[0:2] == ("PASS", "")
    assert g1_eval(g1(3, over={319210: {"cls": "HARNESS-FAIL", "rc": 5}}))[0:2] == ("STOP", "G1-HARNESS") and g1_eval(g1(3, over={319210: {"cls": "DEADLOCK-REPORT", "rc": 1}}))[0:2] == ("STOP", "G1-DEADLOCK")
    assert g1_eval(g1(3, over={319210: {"cls": "CRASH", "rc": 139}}))[0:2] == ("STOP", "G1-CRASH") and g1_eval(g1(3, over={319399: {"cls": "STALL", "rc": 4, "stall_hit": 1, "subtype": "L"}}))[0:2] == ("STOP", "G1-STALL-NONQ")
    assert g1_eval(g1(3, over={319210: {"cls": "TIMEOUT", "rc": 124}}))[0:2] == ("PASS", "+WITH-NONCOMPLETE") and g1_eval(g1(3, over={319210: {"cls": "HERMIT-FAIL", "rc": 125}}))[0:2] == ("PASS", "+WITH-NONCOMPLETE")
    assert g1_eval(g1(3)[:-1])[0] == "MISSING"
    allto = [dict(r, cls="TIMEOUT", rc=124) for r in g1(3)]
    assert g1_eval(allto)[0:2] == ("STOP", "G1-NO-RATED")

    def g2c(hits=3, over=None):
        over = over or {}
        rows = g1(5)
        for i, s in enumerate(G1_SEEDS):
            kw = {"id": "G2C.pn.s%d" % s, "step": "G2C", "arm": "pn", "seed": s}
            if i < hits:
                kw.update({"cls": "HIT", "rc": 3, "prop_false": 1})
            kw.update(over.get(s, {}))
            rows.append(_row(**kw))
        return rows
    k, c, n = g2c_eval(g2c(3))
    assert (k, c) == ("PASS", "") and any("Fisher" in x for x in n)
    assert g2c_eval(g2c(2))[0:2] == ("STOP", "G2C-PU-LOW") and g2c_eval(g2c(3, over={319250: {"cls": "HARNESS-FAIL", "rc": 5}}))[0:2] == ("STOP", "G2C-HARNESS")
    assert g2c_eval(g2c(3, over={319250: {"cls": "CRASH", "rc": 139}}))[0:2] == ("STOP", "G2C-HARNESS") and g2c_eval([r for r in g2c(3) if r["step"] != "G2C"])[0] == "MISSING"
    # G2a / G2b lines
    rows = []
    for s in G2A_SEEDS:
        rows.append(_row(id="G2A.u.s%d" % s, step="G2A", arm="u", seed=s, fp="a%d" % s, syscalls=833))
        rows.append(_row(id="G2A.un.s%d" % s, step="G2A", arm="un", seed=s, fp=("a%d" % s) if s == 319401 else "b", syscalls=800))
    out, done = g2a_lines(rows)
    assert done == 10 and any("G2a: equal fingerprints in 1 of 10" in x for x in out) and any("[33]" in x and "design formula 33 " in x for x in out), out


def _selftest_pause_chain():
    g0s = 319101
    rows = mk_g0(ms=lambda a: 3000 if a == "c" else 2000)
    st, need, left = projected_pause(rows, g0s, 1000, 1000 + 6600)
    assert st == "OK" and abs(need - (80 * 3.0 + 600)) < 1e-9 and left == 6600, (st, need, left)          # part A: 80 runs at the larger A median
    assert projected_pause(rows, g0s, 1000, 1000 + 840)[0] == "OK" and projected_pause(rows, g0s, 1000, 1000 + 839)[0] == "PAUSE"          # need = 840 s exactly
    assert projected_pause([], g0s, 0, 99999)[0] == "PAUSE" and projected_pause(rows, g0s, 1000, 900)[0] == "PAUSE"
    assert projected_pause([r for r in rows if r["arm"] == "u"], g0s, 0, 99999)[0] == "PAUSE"              # a missing arm pauses too
    # chain: exhaustive over the four gated step results of each part, the G2A flag, the gate, the binary change and the pause
    import itertools
    n = 0
    for part in PARTS:
        steps = PART_STEPS[part]
        pre = "GLG-%s-" % part
        for combo in itertools.product(("PASS", "STOP", "MISSING"), repeat=len(steps)):
            for g2a in (False, True):
                for gate in (None, "BUSY"):
                    for bc in (False, True):
                        for pause in (False, True):
                            res = {s: (k, "CODE-" + s if k == "STOP" else ("+L-" + s if k == "PASS" and s in ("g1", "dposh") else "")) for s, k in zip(steps, combo)}
                            v = chain_code(gate, bc, res, pause, g2a, "BUDGET", part)
                            n += 1
                            assert v.startswith(pre), v
                            clean = gate is None and not bc and all(k == "PASS" for k in combo) and (g2a or part == "P") and not pause
                            assert v.startswith(pre + "PASS") == clean, (part, combo, g2a, gate, bc, pause, v)
                            if gate:
                                assert v == pre + "NOT-STARTED/BUSY"
                            elif bc:
                                assert v == pre + "INVALID/BIN-CHANGED"
                            elif "STOP" in combo:
                                assert v == pre + "STOP/CODE-" + steps[combo.index("STOP")], v
                            elif pause and all(k == "PASS" for k in combo[:2]):
                                assert v == pre + "PAUSE/BUDGET"
                            elif not clean:
                                assert v.startswith(pre + "INCONCLUSIVE/"), v
    allp = {s: ("PASS", "") for s in STEPS}
    assert chain_code(None, False, allp, False, True, None, "A") == "GLG-A-PASS" and chain_code(None, False, allp, False, False, None, "P") == "GLG-P-PASS"
    assert chain_code(None, False, {**allp, "g0": ("PASS", "+SHORT-TICKS-U")}, False, True, None, "A") == "GLG-A-PASS+SHORT-TICKS-U"
    assert chain_code(None, False, {**allp, "g0": ("PASS", "+SHORT-TICKS-P"), "g1": ("PASS", "+WITH-NONCOMPLETE")}, False, False, None, "P") == "GLG-P-PASS+SHORT-TICKS-P+WITH-NONCOMPLETE"
    assert chain_code(None, False, allp, False, False, "BUDGET", "A") == "GLG-A-INCONCLUSIVE/g2a-BUDGET"
    assert chain_code(None, False, {**allp, "g2c": ("MISSING", "G2C")}, False, False, None, "P") == "GLG-P-INCONCLUSIVE/g2c-NOT-RUN"
    assert chain_code(None, False, {**allp, "g1": ("STOP", "G1-PU-LOW")}, False, True, None, "A") == "GLG-A-PASS"       # a P step result is not read in part A
    assert chain_code(None, False, {**allp, "dposh": ("STOP", "DPOSH-MISS")}, False, False, None, "P") == "GLG-P-PASS"       # an A step result is not read in part P
    # a step that does not belong to the part is never evaluated, even when its rows would pass (part A makes no P run, part P no planted run)
    g1rows = [_row(id="G1.p.s%d" % s, step="G1", arm="p", seed=s, cls="HIT", rc=3, prop_false=1) for s in G1_SEEDS]
    dprows = [_row(id="DPH.s%d" % s, step="DPH", arm="d", k=PLANT_KS[(s - DPOS_SEEDS[0]) % 5], seed=s, cls="HIT", rc=3, prop_false=1, applied=1, ms=800, subtype="Q+attributed") for s in DPOS_SEEDS]
    assert step_eval("g1", g1rows, 500, 319101, "", no_stderr, "P")[0] == "PASS" and step_eval("g1", g1rows, 500, 319101, "", no_stderr, "A")[0] == "MISSING"
    assert step_eval("dposh", dprows, 500, 319101, "", no_stderr, "A")[0] == "PASS" and step_eval("dposh", dprows, 500, 319101, "", no_stderr, "P")[0] == "MISSING"
    # phase_facts
    g, b, p, r = phase_facts("1\tGATE-FAIL CORES\n2\tBIN-CHANGED condlock-U\n3\tPAUSE-BUDGET need=1\n4\tIDLE-EXHAUSTED: x\n5\tSTOPPED\n")
    assert g == "CORES" and b and p and r == "IDLE-EXHAUSTED"
    assert phase_facts("1\tHALTED: x - no further step\n")[3] == "HALTED" and phase_facts("1\tSIGNAL: the driver was told to stop; HALT written\n")[3] == "SIGNAL"
    assert phase_facts("1\tBUDGET: past the no-new-work line\n")[3] == "BUDGET" and phase_facts("1\tTIMEOUT-HALT x\n")[3] == "TIMEOUT-HALT" and phase_facts("")[3] is None
    return n


U_DUMP = ("condlock: HIT subtype=Q+attributed s_seq=38324 blocked=4 waiting=4 locked=0 progress=76648 stagnant=10 ticks=17 cond=U wseq=58771 "
          "g1_start=58759 g1=0 g_refs=0,8 g_size=0,0 g1_orig_size=8 wrefs>>3=4 wrefs_match=1 g_signals=2,0")


def _selftest_tp_dump_parse():
    good = [dict(id="tp-%s-%s" % k, case=k[0], arm=k[1], rc=0, ms=160, virt_ns=60011973185, turns=34, ok=1) for k in TP_NEED_PART["A"]]
    assert tp_eval(good)[0] == "PASS" and tp_eval(good[:1])[0] == "MISSING" and len(good) == 2 and {g["arm"] for g in good} == {"c"}      # part A rechecks arm C only
    for field, val in (("ok", 0), ("ms", 10000), ("ms", -1), ("virt_ns", 59999999999)):
        bad = [dict(g) for g in good]
        bad[1][field] = val
        assert tp_eval(bad)[0:2] == ("STOP", "TP-FF-FAIL"), (field, val)
    assert tp_eval([dict(g, ms=9999) for g in good])[0] == "PASS"
    assert dump_pattern(U_DUMP) == "DP-SHAPE" and dump_pattern(U_DUMP.replace("g_refs=0,8", "g_refs=1,7")) == "DP-OTHER" and dump_pattern(U_DUMP.replace("cond=U", "cond=C")) == "DP-NA-LAYOUT"
    assert dump_pattern("garbage") == "DP-UNPARSABLE" and dump_pattern(U_DUMP.replace("cond=U", "cond=none")) == "DP-NA-LAYOUT"
    line = "\t".join(str(_row()[c]) for c in COLS)
    assert parse_rows(line + "\nbad\tline\n")[0]["ticks"] == 18 and len(parse_rows(line + "\nshort")) == 1 and parse_rows(line.replace("\t-1\t", "\t-\t", 1))[0]["applied"] == -1
    assert len(COLS) == 31 and len(TP_COLS) == 8
    assert parse_tp("tp-i-c\ti\tc\t0\t160\t60011973185\t34\t1\nshort\n")[0]["virt_ns"] == 60011973185


def selftest():
    _selftest_stats()
    _selftest_classes()
    _selftest_timing_wall_only()
    _selftest_g0()
    _selftest_g0_p()
    _selftest_skid()
    _selftest_planted()
    _selftest_g1()
    _selftest_tp_dump_parse()
    n = _selftest_pause_chain()
    print("selftest ok: statistics anchored, classes, skid signature, timing, G0 codes of both parts, planted steps, G1/G2, TP, dump shape")
    print("selftest ok: %d chain combinations" % n)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    part_ok = lambda p: p in PARTS
    if mode == "gate" and len(sys.argv) == 7 and part_ok(sys.argv[6]):
        step, runs_tsv, rnd, g0s, part = sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]), sys.argv[6]
        tp_text = read_text(os.path.join(os.path.dirname(os.path.abspath(runs_tsv)), "tp.tsv"))
        kind, code, _ = step_eval(step, parse_rows(read_text(runs_tsv)), rnd, g0s, tp_text, stderr_reader(out_dir_of(runs_tsv)), part)
        print("%s %s" % (kind, code) if kind != "MISSING" else "MISSING %s" % step)
    elif mode == "first" and len(sys.argv) == 5 and part_ok(sys.argv[4]):
        print(g0_first(parse_rows(read_text(sys.argv[2])), int(sys.argv[3]), sys.argv[4]))
    elif mode == "skidnext" and len(sys.argv) == 7 and part_ok(sys.argv[6]):
        print(skid_next(parse_rows(read_text(sys.argv[2])), sys.argv[3], int(sys.argv[4]), int(sys.argv[5]), stderr_reader(out_dir_of(sys.argv[2])), sys.argv[6]))
    elif mode == "pause" and len(sys.argv) == 7 and part_ok(sys.argv[6]):
        st, need, left = projected_pause(parse_rows(read_text(sys.argv[2])), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), stderr_reader(out_dir_of(sys.argv[2])), sys.argv[6])
        print("%s need=%s left=%d" % (st, "n/a" if need is None else "%.1f" % need, left))
    elif mode == "verdict" and len(sys.argv) == 3:
        main_verdict(sys.argv[2])
    elif mode == "selftest":
        selftest()
    else:
        sys.exit("usage: glg_eval.py gate STEP RUNS_TSV RND G0S A|P | first RUNS_TSV G0S A|P | skidnext RUNS_TSV ARM SEED G0S A|P | pause RUNS_TSV G0S NOW NEWBY A|P | verdict RUN_DIR | selftest")
