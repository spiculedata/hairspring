#!/usr/bin/env python3
# g_eval.py : the pre-registered rules of the G spec (section 8). Nothing here is tuned after seeing data. Every number is copied from GO-SQLCANCEL-DESIGN.
#   g_eval.py gate STEP RUNS_TSV QW QP G0S   prints one line: "PASS <step>", "STOP <code>" or "MISSING <step>"   (used by g.sh on the box between steps)
#   g_eval.py g0first RUNS_TSV G0S          prints ALLBAD when the first G0 seed's nine runs all failed to complete, else OK   (g.sh stops spending runs on ALLBAD)
#   g_eval.py skidnext RUNS_TSV ARM SEED G0S prints "RUN <spare seed>" when the pair (ARM, SEED) is to be replaced by the SKID_OVERSHOOT rule (spec 5.3a) and the spare's runs are not yet made, else DONE   (g.sh g0_replace)
#   g_eval.py pause RUNS_TSV G0S NOW NEWBY  prints "OK need=<s> left=<s>" or "PAUSE need=<s> left=<s>": the projected-time check before G1 (design 2.5)
# The stderr of a run is read (out/<id>.stderr next to RUNS_TSV) ONLY to recognise the SKID_OVERSHOOT signature of a Hermit failure; nothing else reads it.
#   g_eval.py verdict RUN_DIR               prints the tables and "OUTCOME <code>"   (run on the Mac over the archive copy of $R)
#   g_eval.py selftest                      exhaustive and sample tests, no files written (also run on the box as gate EVAL-SELFTEST)
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sclib  # noqa: E402

ARMS = ("u", "c", "cp")
CONTROLS = ("c", "cp")
G0_BLOCKS = (339511, 339521, 339531, 339541)                     # rev 3: launch 1 uses 339511; a relaunch after G0T-LOWER-* uses the next fresh block (339101 was rev 2's and is spent)
SPARE_PAIRS = {339511: (339561, 339562), 339521: (339563, 339564), 339531: (339565, 339566), 339541: (339567, 339568)}   # the fixed spare list 339561-339569, two per launch (339569 unused)
SKID_MAX_REPLACEMENTS = 2        # spec 5.3a: at most 2 SKID_OVERSHOOT replacements per launch; a third is G0-INCOMPLETE
QW_ALLOWED = (500, 250)                                          # design 2.5: Qw halves from 1000 (rev 2) to 500 (rev 3 launch 1), floor 250
QP_ALLOWED = (100, 50)                                           # design 2.5: P is 100; lowered once (the lowered value is a PROPOSAL, see the spec)
QW_FLOOR, QP_START = 250, 100
G1_SEEDS = list(range(339201, 339401))
G2A_SEEDS = list(range(339401, 339411))
A_MEDIAN_LIMIT_MS = 1800         # design 2.5, 7.2 (rev 4) G0t (A): each arm's median wall at most 1.8 s (C1: 6,000 runs in a 3 h cap)
P_MEDIAN_LIMIT_MS = 4000         # design 2.5, 7.2 (rev 4) G0t (P): the P-u median wall at most 4.0 s (1,000 P runs in about 70 min)
KILL_MS = 30000                  # design 2.5: over 30 s at the floor is a kill
C1_SEEDS_PER_ARM = 2000          # design 2.5: C1 is 6,000 runs = 3 arms x 2,000 seeds; kept whatever the box hours (spec 5.5)
PROJECT_P_RUNS, PROJECT_A_RUNS, PROJECT_MARGIN_S = 1000, 20, 600     # design 2.5: the projected-time check before G1
MIN_DISTINCT_FP = 3              # design 7.2 G0d
MIN_P_HITS = 3                   # design 7.2 G1 (P-u at least 3 of 200); G2c uses the same 3
EXPECTED_SYSCALL_DIFF = 17       # design 7.4 G2b hypothesis (not a gate)
STEPS = ("g0", "g0p", "g1", "g2a", "g2c")
FIRST = {"g0": "G0", "g0p": "G0P", "g1": "G1", "g2a": "G2A", "g2c": "G2C"}


def read_text(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def cls(r):
    return sclib.classify(r, hermit=True)


def sel(rows, step, **kw):
    return [r for r in rows if r["step"] == step and all(r[k] == v for k, v in kw.items())]


def seedlist(g0s):
    return [str(s) for s in range(g0s, g0s + 10)]


# ---------------------------------------------------------------- the SKID_OVERSHOOT replacement rule (spec 5.3a)
SKID_LINE = re.compile(r"^HERMIT_SKID_OVERSHOOT rcb_actual=\d+ rcb_target=\d+ skid_margin=\d+ overshoot=\d+$")
SKID_PANIC_AT = "panicked at detcore/src/lib.rs:348"
SKID_INTERNAL = "HERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)"
SKID_FOREIGN = ("HERMIT_CLEANUP_UNCONFIRMED", "tool_global.rs:1619", "Usage: hermit", "Deadlock detected")


def no_stderr(run_id):
    return ""


def stderr_reader(out_dir):
    """run id -> the text of out_dir/<id>.stderr ('' when it is absent)."""
    return lambda run_id: read_text(os.path.join(out_dir, run_id + ".stderr"))


def out_dir_of(runs_tsv):
    """The out/ directory that sits next to runs.tsv in a work dir (and in its archive copy)."""
    return os.path.join(os.path.dirname(os.path.abspath(runs_tsv)), "out")


def spares_for(g0s):
    """The two spare seeds of this launch's G0 block (a seed outside the table, as the selftest uses, gets the first pair)."""
    return SPARE_PAIRS.get(g0s, SPARE_PAIRS[G0_BLOCKS[0]])


def skid_run(r, text):
    """True when the run is a Hermit failure of the single class SKID_OVERSHOOT: exit status 125 (which sclib.classify reads as HERMIT-FAIL, never as a timeout);
    a HERMIT_SKID_OVERSHOOT rcb line; and nothing else Hermit-marked in stderr (no other infra marker, the panic only at detcore/src/lib.rs:348, the internal
    failure only a container-child exit 101, a task panic only with the skid message); no Go fatal and no deadlock report."""
    if r["rc"] != 125 or r["fatal"] > 0 or r["dead"] > 0:
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
    """One (arm, seed) pair of 3 runs: MISSING (not 3 rows); OK (every run completed); SKID (every run that did not complete is a skid_run);
    OTHER (any other defect, a timeout included). A pair that is SKID and not OTHER is the only kind the rule replaces."""
    if len(rs) != 3:
        return "MISSING"
    bad = [r for r in rs if cls(r) not in sclib.RATED]
    if not bad:
        return "OK"
    return "SKID" if all(skid_run(r, se(r["id"])) for r in bad) else "OTHER"


def g0_effective(rows, g0s, se):
    """The driver's order (seed-major, then arm, then the replacement chain) replayed on the rows.
    -> (eff, notes, pending, nondet): eff[(arm, seed)] = (the runs of the pair that counts, the seed they ran with); notes = one line per replacement;
    pending[(arm, seed)] = the spare seed whose runs have been decided but not yet made; nondet = one line per replaced pair whose completed runs disagree.
    A replaced pair's rows (the failed run and its completed reps) stay in runs.tsv and are not scored as a pair, and are in no other list (G0d, G0c, the timing
    median); but two or more of its completed runs that differ in stdout or fingerprint are G0-NONDET."""
    g0, g0r = sel(rows, "G0"), sel(rows, "G0R")
    spares, used = list(spares_for(g0s)), 0
    eff, notes, pending, nondet = {}, [], {}, []
    for s in seedlist(g0s):
        for arm in ARMS:
            cur, src = s, g0
            while True:
                rs = [r for r in src if r["arm"] == arm and r["seed"] == cur]
                st = pair_state(rs, se)
                if st != "SKID":
                    break
                if used >= SKID_MAX_REPLACEMENTS:
                    notes.append("SKID_OVERSHOOT: A-%s seed %s is not replaced, the %d replacements of this launch are used (G0-INCOMPLETE)" % (arm, cur, SKID_MAX_REPLACEMENTS))
                    break
                nxt = str(spares[used])
                used += 1
                done = [r for r in rs if cls(r) in sclib.RATED]
                so_d, fp_d = len({r["stdout_sha"] for r in done}) > 1, len({r["fp"] for r in done}) > 1
                notes.append("SKID_OVERSHOOT replacement %d of %d: A-%s seed %s -> spare seed %s; raw rows kept, excluded from scoring: %s; its %d completed runs: stdout %s, fingerprint %s" % (
                    used, SKID_MAX_REPLACEMENTS, arm, cur, nxt, ", ".join(r["id"] for r in rs), len(done),
                    "DIFFER" if so_d else "equal", "DIFFER" if fp_d else "equal"))
                if so_d or fp_d:       # two or more completed runs of the replaced pair disagree: a crash in one rep must not hide it (spec 5.3a item 5)
                    nondet.append("G0-NONDET: replaced pair A-%s seed %s: its completed runs differ in %s" % (
                        arm, cur, "stdout and fingerprint" if so_d and fp_d else "stdout" if so_d else "fingerprint"))
                cur, src = nxt, g0r
            if cur != s and st == "MISSING" and not rs:
                pending[(arm, s)] = cur
            eff[(arm, s)] = (rs, cur)
    return eff, notes, pending, nondet


def effective_rows(rows, g0s, se):
    eff = g0_effective(rows, g0s, se)[0]
    return [r for arm in ARMS for s in seedlist(g0s) for r in eff[(arm, s)][0]]


# ---------------------------------------------------------------- G0, G0d, G0c, G0t (A arms)
def g0_eval(rows, g0s, qw, se=no_stderr):
    """-> (kind, code, notes, per) with kind PASS, STOP or MISSING. per[(arm, seed)] = DET | NONDET-* | INCOMPLETE:... | MISSING (by the seed of the block;
    a pair replaced by the SKID_OVERSHOOT rule shows the result of its spare pair, named in the notes)."""
    eff, notes, _, excl_nondet = g0_effective(rows, g0s, se)
    notes += excl_nondet
    per = {}
    g0 = sel(rows, "G0") + sel(rows, "G0R")          # every row, for the control-hit note
    erows = effective_rows(rows, g0s, se)            # the runs that count: replaced pairs are out
    for arm in ARMS:
        for s in seedlist(g0s):
            rs, cur = eff[(arm, s)]
            if len(rs) != 3:
                per[(arm, s)] = "MISSING"
                continue
            bad = sorted({cls(r) for r in rs if cls(r) not in sclib.RATED})
            nofp = any(r["fp"] in ("NOSUMMARY", "-", "") for r in rs)
            if bad or nofp:
                per[(arm, s)] = "INCOMPLETE:" + ",".join(bad or ["NOSUMMARY"])
                continue
            so, fp = len({r["stdout_sha"] for r in rs}), len({r["fp"] for r in rs})
            per[(arm, s)] = "DET" if so == 1 and fp == 1 else "NONDET-" + ("both" if so > 1 and fp > 1 else "stdout" if so > 1 else "fp")
            if len({r["events_sha"] for r in rs}) > 1:
                notes.append("events differ between the 3 runs of %s seed %s (recorded, not a gate)" % (arm, cur))
    for arm in CONTROLS:
        if any(sclib.hit_detail(r) for r in g0 if r["arm"] == arm):
            notes.append("G0 NOTE: a hit in %s (a fixed toolchain) is reported in full, design 4.2 (not a G0 gate)" % arm)
    vals = list(per.values())
    bad_kinds = {k for v in vals if v.startswith("INCOMPLETE:") for k in v[len("INCOMPLETE:"):].split(",")}
    meds = {arm: sclib.median([r["ms"] for r in erows if r["arm"] == arm]) for arm in ARMS if any(r["arm"] == arm for r in erows)}
    notes.append("G0t medians (ms), timeouts counted at their measured wall time, replaced pairs excluded: " + ", ".join("%s=%s" % (a, meds[a]) for a in meds))
    if len(meds) == len(ARMS):
        c1_s = C1_SEEDS_PER_ARM * sum(meds.values()) / 1000.0
        notes.append("C1 projection (not a gate): %d x (%s) = %.0f s = %.2f box hours for the 6,000 counted runs at Qw=%d, if these medians held at C1 scale (design 2.5: 1.8 s per run = 3.00 h)" % (
            C1_SEEDS_PER_ARM, " + ".join("%.3f s" % (meds[a] / 1000.0) for a in ARMS), c1_s, c1_s / 3600.0, qw))
    worst = max(meds.values()) if meds else 0
    # order: a determinism failure; a real failure to complete (not merely slow); the timing rule (a run killed by the cap is slow, not broken); the
    # remaining incompleteness; missing runs; then the diversity and coverage gates
    if any(v.startswith("NONDET") for v in vals) or excl_nondet:
        return "STOP", "G0-NONDET", notes, per
    if bad_kinds - {"TIMEOUT"}:
        return "STOP", ("G0-ALLBAD" if g0_first_allbad(rows, g0s) else "G0-INCOMPLETE"), notes, per
    c1_hours = False
    if worst > A_MEDIAN_LIMIT_MS:
        if qw > QW_FLOOR:
            return "STOP", "G0T-LOWER-QW", notes, per
        if worst > KILL_MS:
            return "STOP", "G0T-KILL", notes, per
        # at the floor and still over 1.8 s (G0T-C1-BUDGET of rev 2): the response fixed in advance is MORE BOX HOURS, not fewer seeds (budget reason only). It is not a stop:
        # the 6,000 counted runs stay, Qw stays at the floor, and the remaining G0 checks and the later gates are read as usual; the label G0T-C1-HOURS is carried to the code.
        c1_hours = True
        notes.append("G0T-C1-BUDGET: a median over 1.8 s at the floor Qw; response fixed in advance: more box hours, the 6,000 counted runs and the R-HIT threshold unchanged")
    if bad_kinds:
        return "STOP", "G0-INCOMPLETE", notes, per
    if any(v == "MISSING" for v in vals):
        return "MISSING", "G0", notes, per
    u = [r for r in erows if r["arm"] == "u"]
    fps = {r["fp"] for r in u if r["rep"] == "1"}
    if len(fps) < MIN_DISTINCT_FP:
        return "STOP", "G0D-NODIV", notes + ["distinct fingerprints among the 10 A-u seeds: %d" % len(fps)], per
    if not any(r["ovlt"] > 0 for r in u):
        return "STOP", "G0C-NO-OVERLAP", notes, per
    if not any(r["errh"] > 0 for r in u):
        return "STOP", "G0C-NO-SCAN-ERROR", notes, per
    return "PASS", ("G0T-C1-HOURS" if c1_hours else "G0"), notes, per


def g0_first_allbad(rows, g0s):
    """True when the first seed's nine runs (3 arms x 3 reps) all failed to complete, for any reason (the F0 'ALL-BAD' precedent: stop spending runs)."""
    first = [r for r in sel(rows, "G0") if r["seed"] == seedlist(g0s)[0]]
    return len(first) == 9 and all(cls(r) not in sclib.RATED for r in first)


def g0p_eval(rows, g0s, qp):
    notes = []
    rs = sel(rows, "G0P")
    if len(rs) != 10:
        return "MISSING", "G0P", notes
    bad = [cls(r) for r in rs if cls(r) not in sclib.RATED]
    med = sclib.median([r["ms"] for r in rs])
    notes.append("G0t P-u median (ms), timeouts counted at their measured wall time: %s; classes not rated: %s" % (med, sorted(set(bad))))
    if set(bad) - {"TIMEOUT"}:
        return "STOP", "G0P-INCOMPLETE", notes
    if med > P_MEDIAN_LIMIT_MS:
        return "STOP", ("G0T-LOWER-QP" if qp > min(QP_ALLOWED) else "G0T-QP-SLOW-AFTER-LOWERING"), notes
    if bad:
        return "STOP", "G0P-INCOMPLETE", notes
    return "PASS", "G0P", notes


def projected_pause(rows, g0s, now_s, newby_s, se=no_stderr):
    """The projected-time check before G1 (design 2.5): 1,000 x (P-u median) + 20 x (A-u median) + 600 s against the time left before the no-new-runs line.
    -> (pause, need_s, left_s). Medians are the G0P and the A-u G0 medians in ms (replaced pairs excluded); nothing here is read from G1."""
    pm = sclib.median([r["ms"] for r in sel(rows, "G0P")])
    am = sclib.median([r["ms"] for r in effective_rows(rows, g0s, se) if r["arm"] == "u"])
    if pm is None or am is None:
        return True, None, newby_s - now_s       # no medians, no projection: stop rather than spend the window blind
    need = PROJECT_P_RUNS * pm / 1000.0 + PROJECT_A_RUNS * am / 1000.0 + PROJECT_MARGIN_S
    return need > newby_s - now_s, need, newby_s - now_s


# ---------------------------------------------------------------- G1
def counts(rs):
    c = {k: 0 for k in sclib.CLASSES}
    for r in rs:
        c[cls(r)] += 1
    c["rated"] = c["HIT"] + c["NOHIT"]
    return c


def g1_eval(rows):
    rs = sel(rows, "G1")
    notes = []
    per = {a: [r for r in rs if r["arm"] == a] for a in ARMS}
    if any(len(per[a]) != len(G1_SEEDS) for a in ARMS):
        return "MISSING", "G1", notes, {}
    c = {a: counts(per[a]) for a in ARMS}
    for a in ARMS:
        notes.append("G1 P-%s: %s | not rated: timeout=%d crash=%d hermit=%d harness=%d" % (
            a, sclib.rate_text(c[a]["HIT"], c[a]["rated"]), c[a]["TIMEOUT"], c[a]["CRASH"], c[a]["HERMIT-FAIL"], c[a]["HARNESS-FAIL"]))
    if sum(c[a]["HARNESS-FAIL"] for a in ARMS) > 0:
        return "STOP", "G1-HARNESS", notes, c
    if any(c[a]["rated"] == 0 for a in ARMS):
        return "STOP", "G1-NO-RATED", notes, c       # a zero over zero rated runs says nothing
    if sum(c[a]["HIT"] for a in CONTROLS) > 0 or any(sclib.hit_detail(r) for a in CONTROLS for r in per[a]):
        return "STOP", "G1-CONTROL-HIT", notes, c
    if any(r["fnext"] > 0 or r["handt"] > 0 for a in CONTROLS for r in per[a] if cls(r) in sclib.RATED):
        return "STOP", "G1-CONTROL-MECHANISM", notes, c
    if c["u"]["HIT"] < MIN_P_HITS:
        return "STOP", "G1-PU-LOW", notes, c
    other = sum(c[a][k] for a in ARMS for k in ("TIMEOUT", "CRASH", "HERMIT-FAIL"))
    return "PASS", "G1" + ("-WITH-NONCOMPLETE" if other else ""), notes, c


# ---------------------------------------------------------------- G2c (the detection gate); G2a/G2b (recorded)
def g2c_eval(rows):
    notes = []
    rs = sel(rows, "G2C")
    per = {a: [r for r in rs if r["arm"] == a] for a in ("u", "cp")}
    if any(len(per[a]) != len(G1_SEEDS) for a in per):
        return "MISSING", "G2C", notes, {}
    g1 = {a: counts(sel(rows, "G1", arm=a)) for a in ("u", "cp")}
    vo = {a: counts(per[a]) for a in per}
    for a in per:
        notes.append("G2c violation-only P-%s: %s | shim G1 P-%s: %d/%d | Fisher two-sided p = %.6g" % (
            a, sclib.rate_text(vo[a]["HIT"], vo[a]["rated"]), a, g1[a]["HIT"], g1[a]["rated"],
            sclib.fisher_two_sided(vo[a]["HIT"], vo[a]["rated"] - vo[a]["HIT"], g1[a]["HIT"], g1[a]["rated"] - g1[a]["HIT"])))
    if sum(vo[a]["HARNESS-FAIL"] for a in vo) > 0:
        return "STOP", "G2C-HARNESS", notes, vo
    if any(vo[a]["rated"] == 0 for a in vo):
        return "STOP", "G2C-NO-RATED", notes, vo
    if vo["u"]["HIT"] < MIN_P_HITS:
        return "STOP", "G2C-PU-LOW", notes, vo
    if vo["cp"]["HIT"] > 0:
        return "STOP", "G2C-CONTROL-HIT", notes, vo
    return "PASS", "G2C", notes, vo


def g2a_eval(rows):
    """Recorded, never a gate: per seed, fingerprints equal? syscall difference? Returns (kind, lines)."""
    rs = sel(rows, "G2A")
    lines, eq, diffs, n = [], 0, [], 0
    for s in G2A_SEEDS:
        a = [r for r in rs if r["seed"] == str(s) and r["build"] == "s"]
        b = [r for r in rs if r["seed"] == str(s) and r["build"] == "v"]
        if len(a) != 1 or len(b) != 1:
            lines.append("G2a seed %d: MISSING" % s)
            continue
        a, b = a[0], b[0]
        n += 1
        same = a["fp"] == b["fp"] and a["fp"] not in ("NOSUMMARY", "-")
        eq += 1 if same else 0
        try:
            d = int(a["syscalls"]) - int(b["syscalls"])
        except ValueError:
            d = None
        nohit = not sclib.hit_detail(a) and not sclib.hit_detail(b)
        diffs.append((d, nohit))
        lines.append("G2a seed %d: fingerprints %s; syscalls shim-vo = %s; classes %s/%s; hits %s/%s" % (
            s, "EQUAL" if same else "differ", d, cls(a), cls(b), sclib.hit_detail(a), sclib.hit_detail(b)))
    on = [d for d, ok in diffs if ok and d is not None]
    lines.append("G2a (recorded, not a gate; design expectation 0 of 10 equal): %d of %d seeds with equal fingerprints" % (eq, n))
    lines.append("G2b (recorded, not a gate; hypothesis: %d on every seed where neither build hit): %d of %d such seeds had exactly %d; raw differences %s" % (
        EXPECTED_SYSCALL_DIFF, sum(1 for d in on if d == EXPECTED_SYSCALL_DIFF), len(on), EXPECTED_SYSCALL_DIFF, sorted(d for d, _ in diffs if d is not None)))
    return ("MISSING" if n < len(G2A_SEEDS) else "DONE"), lines


# ---------------------------------------------------------------- the chain and the verdict
def step_results(rows, args, se=no_stderr):
    qw, qp, g0s = args
    out = {}
    out["g0"] = g0_eval(rows, g0s, qw, se)[:2]
    out["g0p"] = g0p_eval(rows, g0s, qp)[:2]
    out["g1"] = g1_eval(rows)[:2]
    out["g2a"] = ("PASS" if g2a_eval(rows)[0] == "DONE" else "MISSING", "G2A")      # not a gate: PASS here means 'ran'
    out["g2c"] = g2c_eval(rows)[:2]
    return out


def gate_text(step, rows, args, se=no_stderr):
    r = step_results(rows, args, se)[step]
    return {"PASS": "PASS %s", "STOP": "STOP %s", "MISSING": "MISSING %s"}[r[0]] % r[1]


def chain_code(gate, results, halt_reason, bin_changed=False, pause=False):
    """results: step -> (kind, code). First rule that holds; always returns a G- code. The chain runs g0, g0p, the projected-time check, g1, g2a, g2c in that order."""
    if gate:
        return "G-NOT-STARTED/%s" % gate
    if bin_changed:
        return "G-INVALID/BIN-CHANGED"
    for step in STEPS:
        if step == "g1" and pause:
            return "G-PAUSE/BUDGET"      # a stop, not a failure: G1 was not started because the window cannot hold it
        kind, code = results[step]
        if kind == "STOP":
            return "G-STOP/%s" % code
        if kind == "MISSING":
            return "G-INCONCLUSIVE/%s-%s" % (FIRST[step], halt_reason or "NOT-RUN")
    code = results["g1"][1].replace("G1", "G-PASS", 1)
    return code + ("-C1-HOURS" if results["g0"][1] == "G0T-C1-HOURS" else "")      # a G0 that passed at the floor Qw over 1.8 s (more box hours)


def steps_facts(text):
    gate, halt, args, bin_changed, pause = None, None, None, False, False
    for line in text.splitlines():
        n = line.split("\t", 1)[1] if "\t" in line else line
        m = re.match(r"GATE-FAIL (\S+)", n)
        if m:
            gate = m.group(1)
        if n.startswith("BIN-CHANGED"):
            bin_changed = True
        if n.startswith("PAUSE-BUDGET"):
            pause = True
        m = re.match(r"ARGS qw=(\d+) qp=(\d+) g0s=(\d+)", n)
        if m:
            args = tuple(int(x) for x in m.groups())
        for key in ("BUDGET", "HALTED", "SIGNAL", "IDLE-EXHAUSTED", "REFUSED", "SIZE-CAP", "STOPPED", "TIMEOUT-HALT"):
            if n.startswith(key) and halt is None:
                halt = key
    return gate, halt, args, bin_changed, pause


def read_json_outcome(path):
    try:
        return json.loads(read_text(path))[0]["outcome"]
    except Exception:
        return None


def main_verdict(run_dir):
    rows = sclib.parse_rows(read_text(os.path.join(run_dir, "runs.tsv")))
    gate, halt, args, bin_changed, pause = steps_facts(read_text(os.path.join(run_dir, "steps.tsv")))
    print("gate     :", gate or "none", "| args (QW, QP, G0S):", args, "| halt reason:", halt or "none", "| binary changed:", bin_changed, "| projected-time pause:", pause)
    if args is None:
        print("OUTCOME", chain_code(gate or "NO-ARGS", {s: ("MISSING", "") for s in STEPS}, halt, bin_changed, pause))
        return
    qw, qp, g0s = args
    se = stderr_reader(os.path.join(run_dir, "out"))
    k, code, notes, per = g0_eval(rows, g0s, qw, se)
    print("G0 (design 7.2):", k, code)
    for arm in ARMS:
        print("  A-%-2s per seed: %s" % (arm, " ".join("%s=%s" % (s[-2:], per.get((arm, s), "MISSING")) for s in seedlist(g0s))))
    for n in notes:
        print("  ", n)
    k, code, notes = g0p_eval(rows, g0s, qp)
    print("G0t P-u (design 7.2):", k, code)
    for n in notes:
        print("  ", n)
    k, code, notes, _ = g1_eval(rows)
    print("G1 (design 7.2):", k, code)
    for n in notes:
        print("  ", n)
    k, code, notes, _ = g2c_eval(rows)
    print("G2c (design 7.4):", k, code)
    for n in notes:
        print("  ", n)
    k, lines = g2a_eval(rows)
    print("G2a/G2b:", k)
    for n in lines:
        print("  ", n)
    for ge in ("GE1", "GE3", "GE4"):
        o = read_json_outcome(os.path.join(run_dir, "out", ge + ".out"))
        print("GE %s (informational): %s" % (ge, "no JSON" if o is None else {k: o.get(k) for k in ("status", "trials", "inconclusive", "distinct_schedules", "work", "stopped_at_first_failure")}))
        if o and o.get("failure"):
            print("   failure message:", (o["failure"].get("message") or "")[:300])
    ge2 = read_text(os.path.join(run_dir, "out", "GE2.out"))
    print("GE GE2 (informational, replay of the GE1 bundle): rc %s, %s" % (read_text(os.path.join(run_dir, "out", "GE2.rc")).strip() or "none",
          "; ".join(l.strip() for l in ge2.splitlines() if "matches the recording" in l or "reproduced" in l) or "no replay lines"))
    allc = {a: counts(sel(rows, "G1", arm=a)) for a in ARMS}
    print("not-rated classes in G1 (timeout/crash/hermit/harness):", {a: (allc[a]["TIMEOUT"], allc[a]["CRASH"], allc[a]["HERMIT-FAIL"], allc[a]["HARNESS-FAIL"]) for a in ARMS})
    hf = {}
    for r in rows:
        if cls(r) == "HERMIT-FAIL":
            key = (r["step"], r["arm"])
            n_all, n_skid = hf.get(key, (0, 0))
            hf[key] = (n_all + 1, n_skid + (1 if skid_run(r, se(r["id"])) else 0))
    print("Hermit failures (HERMIT-FAIL) by (step, arm) as (all, of them SKID_OVERSHOOT), every row raw; counted per arm as R-CRASH outside the rated runs by the later C1 spec:", hf or "none")
    print("OUTCOME", chain_code(gate, step_results(rows, args, se), halt, bin_changed, pause))


# ---------------------------------------------------------------- selftest
def _row(step, arm, seed, rep=1, build="s", mode="std", **kw):
    r = sclib._blank(step=step, arm=arm, seed=str(seed), rep=str(rep), build=build, mode=mode, id="%s.%s.%s.%s.%s" % (step, arm, build, seed, rep))
    r.update({"stdout_sha": "s%s" % seed, "events_sha": "e%s" % seed, "fp": "f%s" % seed, "syscalls": "100", "ms": 1000})
    r.update(kw)
    return r


def _good_rows(g0s=339101, uhits=8):
    rows = []
    for arm in ARMS:
        for i, s in enumerate(range(g0s, g0s + 10)):
            for rep in (1, 2, 3):
                rows.append(_row("G0", arm, s, rep, ovlt=1 if i == 0 else 0, errh=1 if i == 0 else 0, fp="f%s%s%s" % (arm, s, i % 4), stdout_sha="s%s%s" % (arm, s)))
    rows += [_row("G0P", "u", s, 1, mode="yield") for s in range(g0s, g0s + 10)]
    for s in G1_SEEDS:
        rows.append(_row("G1", "u", s, mode="yield", pf=4 if s - 339201 < uhits else 0, fnext=5, handt=1))
        rows += [_row("G1", a, s, mode="yield", fnext=0, handt=0) for a in CONTROLS]
        rows.append(_row("G2C", "u", s, build="v", mode="yield", vprop=1 if s - 339201 < uhits else 0))
        rows.append(_row("G2C", "cp", s, build="v", mode="yield"))
    for s in G2A_SEEDS:
        rows.append(_row("G2A", "u", s, build="s", fp="a%d" % s, syscalls="117"))
        rows.append(_row("G2A", "u", s, build="v", fp="b%d" % s, syscalls="100"))
    return rows


# The Hermit part of the stderr of the one failed run of rev 2 (the workload's own event lines are left out): the fixture of the SKID_OVERSHOOT signature.
_SKID_ERR = """hermit: virtual-time epoch=2026-01-01T00:00:00+00:00 source=explicit; reproduce with --epoch=2026-01-01T00:00:00+00:00
2026-10-17T01:07:47.398343Z  WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL signum=5 effect=handler-installed-but-never-invoked reason=ptrace consumes every SIGTRAP (syscall/seccomp stops, breakpoints)
HERMIT_SKID_OVERSHOOT rcb_actual=8482180 rcb_target=8482179 skid_margin=25 overshoot=1
2026-10-17T01:08:03.505527Z  WARN reverie_ptrace::timer: Precise timer interrupt arrived after target: actual 8482180 > target 8482179; delivering timer event at the observed counter

thread 'main' (1) panicked at detcore/src/lib.rs:348:9:
HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot! Clock_value: 8482180. Stepped forward 2162 RCBs, but should have trapped at 2161
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
HERMIT_TASK_PANIC tid=7 exit=101 message=HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot! Clock_value: 8482180. Stepped forward 2162 RCBs, but should have trapped at 2161
HERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)
Error: Sandbox container exited unexpectedly: Exited(101)
"""


def _skid_case(failed, g0s=339101, spare_ovl=0, text=_SKID_ERR, **hurt_kw):
    """_good_rows with the runs in `failed` ((arm, seed, rep) as strings) turned into Hermit failures (rc 125, no summary, `text` as their stderr), and the G0R rows
    that the driver would have made for them, in the driver's order and with the two spare seeds of the launch. -> (rows, se)."""
    rows, texts, spares, used = _good_rows(g0s), {}, list(spares_for(g0s)), 0

    def hurt(r):
        r.update(rc=125, hinfra=1, hpanic=1, fp="NOSUMMARY", syscalls="NA", ms=16159)
        r.update(hurt_kw)
        texts[r["id"]] = text

    for r in rows:
        if r["step"] == "G0" and (r["arm"], r["seed"], r["rep"]) in failed:
            hurt(r)
    for s in seedlist(g0s):
        for arm in ARMS:
            cur = s
            while any((arm, cur, str(k)) in failed for k in (1, 2, 3)) and used < SKID_MAX_REPLACEMENTS:
                cur, used = str(spares[used]), used + 1
                for k in (1, 2, 3):
                    x = _row("G0R", arm, cur, k, fp="f%s%s" % (arm, cur), stdout_sha="s%s%s" % (arm, cur), ovlt=spare_ovl, errh=spare_ovl)
                    if (arm, cur, str(k)) in failed:
                        hurt(x)
                    rows.append(x)
    return rows, (lambda rid: texts.get(rid, ""))


def selftest():
    sclib.selftest()
    args = (1000, 100, 339101)
    ok = lambda rows, a=args: step_results(rows, a)
    rows = _good_rows()
    r = ok(rows)
    assert all(r[s][0] == "PASS" for s in STEPS), r
    assert chain_code(None, r, None) == "G-PASS"
    assert gate_text("g0", rows, args) == "PASS G0" and gate_text("g1", rows, args) == "PASS G1"
    k, lines = g2a_eval(rows)
    assert k == "DONE" and "0 of 10 seeds with equal fingerprints" in lines[-2] and "10 of 10 such seeds had exactly 17" in lines[-1], lines[-2:]
    def with_(pred, **kw):
        out = []
        for x in _good_rows():
            x = dict(x)
            if pred(x):
                x.update(kw)
            out.append(x)
        return out

    # G0 failure modes
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "c" and x["seed"] == "339103" and x["rep"] == "2", stdout_sha="other"))["g0"] == ("STOP", "G0-NONDET")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "u" and x["seed"] == "339103" and x["rep"] == "3", fp="other"))["g0"] == ("STOP", "G0-NONDET")
    assert ok(with_(lambda x: x["step"] == "G0" and x["seed"] == "339104" and x["rep"] == "1", rc=125, hinfra=1))["g0"] == ("STOP", "G0-INCOMPLETE")
    assert ok(with_(lambda x: x["step"] == "G0" and x["seed"] == "339101", rc=2, fatal=1))["g0"] == ("STOP", "G0-ALLBAD")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "u", fp="same"))["g0"] == ("STOP", "G0D-NODIV")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "u", ovlt=0))["g0"] == ("STOP", "G0C-NO-OVERLAP")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "u", errh=0))["g0"] == ("STOP", "G0C-NO-SCAN-ERROR")
    # G0t (A): each arm's median at most 1.8 s (design rev 4); over: halve Qw; at the floor over 1.8 s: G0T-C1-BUDGET; at the floor over 30 s: kill
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=1801))["g0"] == ("STOP", "G0T-LOWER-QW")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=1800))["g0"] == ("PASS", "G0")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "u", ms=1801))["g0"] == ("STOP", "G0T-LOWER-QW")     # any arm, not just one
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=10001))["g0"] == ("STOP", "G0T-LOWER-QW")    # the old 10 s line no longer matters
    slow = with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=20000)
    # rev 3: at the floor over 1.8 s the old G0T-C1-BUDGET stop is a pass labelled G0T-C1-HOURS (more box hours); the other G0 checks still apply
    assert step_results(slow, (500, 100, 339101))["g0"] == ("STOP", "G0T-LOWER-QW") and step_results(slow, (250, 100, 339101))["g0"] == ("PASS", "G0T-C1-HOURS")
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "c", ms=1801), (250, 100, 339101))["g0"] == ("PASS", "G0T-C1-HOURS")
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "c", ms=1800), (250, 100, 339101))["g0"] == ("PASS", "G0")
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=30000), (250, 100, 339101))["g0"] == ("PASS", "G0T-C1-HOURS")
    assert step_results(with_(lambda x: x["step"] == "G0", ms=4000, ovlt=0), (250, 100, 339101))["g0"] == ("STOP", "G0C-NO-OVERLAP")      # the label never hides a failed G0 check
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "c" and x["seed"] == "339103" and x["rep"] == "2", ms=4000, stdout_sha="other"), (250, 100, 339101))["g0"] == ("STOP", "G0-NONDET")
    assert step_results(with_(lambda x: x["step"] == "G0", ms=4000), (250, 100, 339101))["g0"] == ("PASS", "G0T-C1-HOURS")
    slow4 = [dict(x, rc=124, ms=60000) if x["step"] == "G0" and x["seed"] == "339101" else x for x in with_(lambda x: x["step"] == "G0", ms=4000)]
    assert step_results(slow4, (250, 100, 339101))["g0"] == ("STOP", "G0-INCOMPLETE")   # rev 3: a timeout at the floor is incompleteness (rev 2 read the budget stop first)
    hrs = g0_eval(with_(lambda x: x["step"] == "G0", ms=4000), 339101, 250)[2]
    assert any("= 24000 s = 6.67 box hours" in n and "Qw=250" in n for n in hrs) and any("G0T-C1-BUDGET" in n and "more box hours" in n for n in hrs), hrs
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=30001), (250, 100, 339101))["g0"] == ("STOP", "G0T-KILL")
    assert step_results(with_(lambda x: x["step"] == "G0" and x["arm"] == "cp", ms=30001), (500, 100, 339101))["g0"] == ("STOP", "G0T-LOWER-QW")
    assert ok([x for x in rows if not (x["step"] == "G0" and x["seed"] == "339109")])["g0"] == ("MISSING", "G0")
    # a run killed by the cap is slow, not broken: all-timeout G0 is a timing result (lower Qw, or kill at the floor); a few timeouts with a fine median are incomplete
    allto = with_(lambda x: x["step"] == "G0", rc=124, ms=60000)
    assert step_results(allto, (1000, 100, 339101))["g0"] == ("STOP", "G0T-LOWER-QW") and step_results(allto, (250, 100, 339101))["g0"] == ("STOP", "G0T-KILL")
    assert ok(with_(lambda x: x["step"] == "G0" and x["seed"] == "339101", rc=124, ms=60000))["g0"] == ("STOP", "G0-INCOMPLETE")
    assert ok(with_(lambda x: x["step"] == "G0" and x["seed"] == "339101" and x["rep"] == "1" and x["arm"] == "c", rc=2, fatal=1, ms=20000))["g0"] == ("STOP", "G0-INCOMPLETE")
    firstto = [x for x in allto if x["seed"] == "339101" or x["step"] != "G0"]
    assert g0_first_allbad(firstto, 339101) and not g0_first_allbad(rows, 339101)
    assert step_results(firstto, (1000, 100, 339101))["g0"] == ("STOP", "G0T-LOWER-QW")      # the driver stops after seed 1; the rest of G0 is MISSING but the timing rule reads first
    assert ok([x for x in rows if not (x["step"] == "G0" and x["seed"] != "339101")])["g0"] == ("MISSING", "G0")
    assert ok(with_(lambda x: x["step"] == "G0" and x["arm"] == "c", vprop=0, pf=2))["g0"] == ("PASS", "G0")          # a control hit at G0 is a note, not a G0 stop
    assert any("fixed toolchain" in n for n in g0_eval(with_(lambda x: x["step"] == "G0" and x["arm"] == "c", pf=2), 339101, 1000)[2])
    k, code, notes, per = g0_eval(with_(lambda x: x["step"] == "G0" and x["arm"] == "u" and x["seed"] == "339102" and x["rep"] == "2", events_sha="zz"), 339101, 1000)
    assert code == "G0" and any("events differ" in n for n in notes) and per[("u", "339102")] == "DET"
    # the SKID_OVERSHOOT replacement rule (spec 5.3a)
    sp1, sp2 = (str(x) for x in spares_for(339101))
    assert (sp1, sp2) == ("339561", "339562") and spares_for(339541) == (339567, 339568) and spares_for(339511) == (339561, 339562)
    flat = [x for pair in SPARE_PAIRS.values() for x in pair]
    assert len(set(flat)) == len(flat) == 8 and all(339561 <= x <= 339569 for x in flat) and not set(flat) & {b + k for b in G0_BLOCKS for k in range(10)}
    se_of = lambda se: (lambda rows, a=args: step_results(rows, a, se))
    # (a) one skid run: its pair is replaced by the first spare and the rule says PASS when the pair's two completed runs agree; the pair is out of the timing
    rows_a, se_a = _skid_case({("u", "339102", "1")})
    for x in rows_a:
        if x["step"] == "G0" and x["arm"] == "u" and x["seed"] == "339102" and x["rep"] in ("2", "3"):
            x["ms"] = 99999                                               # a slow median if the replaced pair still counted (it cannot move a median of 30 alone: see rows_p)
    assert se_of(se_a)(rows_a)["g0"] == ("PASS", "G0") and ok(rows_a)["g0"] == ("STOP", "G0-INCOMPLETE")       # without the stderr text the signature is absent: a plain Hermit failure
    k, code, notes, per = g0_eval(rows_a, 339101, 1000, se_a)
    assert (k, code) == ("PASS", "G0") and per[("u", "339102")] == "DET" and sum(1 for n in notes if "replacement" in n) == 1
    assert any("replacement 1 of 2: A-u seed 339102 -> spare seed 339561" in n and "G0.u.s.339102.1" in n and "raw rows kept" in n for n in notes), notes
    assert any("its 2 completed runs: stdout equal, fingerprint equal" in n for n in notes), notes      # the excluded pair is still described, raw
    assert g0_effective(rows_a, 339101, se_a)[2] == {} and len(effective_rows(rows_a, 339101, se_a)) == 90
    assert g0_effective([x for x in rows_a if x["step"] != "G0R"], 339101, se_a)[2] == {("u", "339102"): sp1}      # what the driver asks for before the spare runs
    assert se_of(se_a)([x for x in rows_a if x["step"] != "G0R"])["g0"] == ("MISSING", "G0")
    assert projected_pause(rows_a, 339101, 0, 1620, se_a)[1] == 1620.0                           # the A-u median leaves the replaced pair out
    rows_p, se_p = _skid_case({("u", "339102", "1")})
    for x in rows_p:
        if x["step"] == "G0" and x["arm"] == "u" and x["seed"] in ("339101", "339102", "339103", "339104", "339105"):
            x["ms"] = max(x["ms"], 3000)                                  # 15 slow A-u runs, 3 of them in the replaced pair: only the median of the other 27 plus the spare pair is 1.0 s
    assert projected_pause(rows_p, 339101, 0, 1620, se_p)[1] == 1620.0 and projected_pause(rows_p, 339101, 0, 1620)[1] == 1640.0
    assert se_of(se_p)(rows_p)["g0"] == ("PASS", "G0")      # the A-u median of the effective runs is 1.0 s; with the replaced pair counted it would be 2.0 s: G0T-LOWER-QW
    assert g0_effective([x for x in rows_a if not (x["step"] == "G0R" and x["rep"] == "3")], 339101, se_a)[2] == {}      # a half-made spare pair is not asked for again
    # (a2) rev 3a: a replaced pair's completed runs must still agree. A crash in one rep (the skid run) must not hide a disagreement between the others.
    for field, what in (("stdout_sha", "stdout"), ("fp", "fingerprint")):
        rows_n, se_n = _skid_case({("c", "339103", "1")})
        for x in rows_n:
            if x["step"] == "G0" and x["arm"] == "c" and x["seed"] == "339103" and x["rep"] == "2":
                x[field] = "other"
        k, code, notes, per = g0_eval(rows_n, 339101, 1000, se_n)
        assert (k, code) == ("STOP", "G0-NONDET") and any(n == "G0-NONDET: replaced pair A-c seed 339103: its completed runs differ in %s" % what for n in notes), (what, notes)
        assert se_of(se_n)(rows_n)["g0"] == ("STOP", "G0-NONDET")
    rows_n, se_n = _skid_case({("c", "339103", "1")})
    for x in rows_n:
        if x["step"] == "G0" and x["arm"] == "c" and x["seed"] == "339103" and x["rep"] == "3":
            x["stdout_sha"], x["fp"] = "o1", "o2"
    assert any("differ in stdout and fingerprint" in n for n in g0_eval(rows_n, 339101, 1000, se_n)[2])
    rows_n, se_n = _skid_case({("c", "339103", "1"), ("c", "339103", "2")})      # two crashed, one completed: nothing to compare, still one replacement and a pass
    assert se_of(se_n)(rows_n)["g0"] == ("PASS", "G0") and not g0_effective(rows_n, 339101, se_n)[3]
    rows_n, se_n = _skid_case({("u", "339102", "1")})                           # rep 3 alone differs from rep 2 (rep 1 crashed)
    for x in rows_n:
        if x["step"] == "G0" and x["arm"] == "u" and x["seed"] == "339102" and x["rep"] == "3":
            x["stdout_sha"] = "other"
    assert se_of(se_n)(rows_n)["g0"] == ("STOP", "G0-NONDET")
    # (b) the spare pair's own facts and fingerprints are the ones that count (G0c and G0d read the effective runs)
    rows_b0, se_b0 = _skid_case({("u", "339101", "2")}, spare_ovl=0)
    rows_b1, se_b1 = _skid_case({("u", "339101", "2")}, spare_ovl=1)
    assert se_of(se_b0)(rows_b0)["g0"] == ("STOP", "G0C-NO-OVERLAP") and se_of(se_b1)(rows_b1)["g0"] == ("PASS", "G0")
    # (c) a pair with two failed runs is still one replacement; two replacements pass, a third is G0-INCOMPLETE
    rows_c, se_c = _skid_case({("c", "339104", "1"), ("c", "339104", "3")})
    assert se_of(se_c)(rows_c)["g0"] == ("PASS", "G0") and sum(1 for n in g0_eval(rows_c, 339101, 1000, se_c)[2] if "replacement" in n) == 1
    rows_d, se_d = _skid_case({("u", "339102", "1"), ("c", "339104", "3")})
    k, code, notes, per = g0_eval(rows_d, 339101, 1000, se_d)
    assert (k, code) == ("PASS", "G0") and any("replacement 2 of 2: A-c seed 339104 -> spare seed 339562" in n for n in notes)
    rows_e, se_e = _skid_case({("u", "339102", "1"), ("c", "339104", "3"), ("cp", "339106", "2")})
    k, code, notes, per = g0_eval(rows_e, 339101, 1000, se_e)
    assert (k, code) == ("STOP", "G0-INCOMPLETE") and per[("cp", "339106")] == "INCOMPLETE:HERMIT-FAIL" and any("is not replaced" in n for n in notes)
    assert g0_effective(rows_e, 339101, se_e)[2] == {}
    # (d) a spare that fails the same way is replaced by the next spare (still two in all); a third failure anywhere stops
    rows_f, se_f = _skid_case({("u", "339102", "1"), ("u", sp1, "2")})
    k, code, notes, per = g0_eval(rows_f, 339101, 1000, se_f)
    assert (k, code) == ("PASS", "G0") and any("replacement 2 of 2: A-u seed %s -> spare seed %s" % (sp1, sp2) in n for n in notes), notes
    assert g0_effective([x for x in rows_f if not (x["step"] == "G0R" and x["seed"] == sp2)], 339101, se_f)[2] == {("u", "339102"): sp2}
    rows_g, se_g = _skid_case({("u", "339102", "1"), ("u", sp1, "2"), ("u", sp2, "3")})
    assert se_of(se_g)(rows_g)["g0"] == ("STOP", "G0-INCOMPLETE")
    rows_h, se_h = _skid_case({("u", "339102", "1"), ("u", sp1, "2"), ("cp", "339107", "1")})
    assert se_of(se_h)(rows_h)["g0"] == ("STOP", "G0-INCOMPLETE")
    # (e) the spare pair is held to the same rules: nondeterminism there is G0-NONDET
    rows_i, se_i = _skid_case({("u", "339102", "1")})
    for x in rows_i:
        if x["step"] == "G0R" and x["rep"] == "3":
            x["fp"] = "other"
    assert se_of(se_i)(rows_i)["g0"] == ("STOP", "G0-NONDET")
    # (f) what is NOT the skid class stays a plain Hermit failure and is not replaced
    base = _SKID_ERR
    mutants = {
        "no rcb line": base.replace("HERMIT_SKID_OVERSHOOT rcb_actual=8482180 rcb_target=8482179 skid_margin=25 overshoot=1\n", ""),
        "rcb line damaged": base.replace("overshoot=1\n", "overshoot=x\n"),
        "cleanup unconfirmed": base + "HERMIT_CLEANUP_UNCONFIRMED pid=1\n",
        "tool_global": base + "tool_global.rs:1619 something\n",
        "usage": base + "Usage: hermit run\n",
        "deadlock": base + "Deadlock detected\n",
        "other internal class": base.replace("class=container-child-exit status=Exited(101)", "class=sandbox-setup status=Exited(1)"),
        "second internal failure": base + "HERMIT_INTERNAL_FAILURE class=container-child-exit status=Signaled(9)\n",
        "panic elsewhere": base.replace("detcore/src/lib.rs:348", "detcore/src/lib.rs:349"),
        "second panic": base + "thread 'x' panicked at reverie/src/other.rs:1:1:\n",
        "other task panic": base + "HERMIT_TASK_PANIC tid=8 exit=101 message=something else\n",
    }
    for name, txt in mutants.items():
        rs, se = _skid_case({("u", "339102", "1")}, text=txt)
        assert se_of(se)(rs)["g0"] == ("STOP", "G0-INCOMPLETE"), name
        assert g0_effective(rs, 339101, se)[2] == {}, name
    for name, kw in (("exit 101", {"rc": 101}), ("fatal marker", {"fatal": 1}), ("deadlock fact", {"dead": 1}), ("exit 2", {"rc": 2}), ("timeout", {"rc": 124})):
        rs, se = _skid_case({("u", "339102", "1")}, **kw)
        assert se_of(se)(rs)["g0"] == ("STOP", "G0-INCOMPLETE"), name
    rs, se = _skid_case({("u", "339102", "1")})
    for x in rs:
        if x["step"] == "G0" and x["arm"] == "u" and x["id"].endswith("339102.2"):
            x.update(rc=2, fatal=1)                                       # another defect in the same pair: the pair is not skid-only
    assert se_of(se)(rs)["g0"] == ("STOP", "G0-INCOMPLETE") and g0_effective(rs, 339101, se)[1] == []
    ok_pair = [x for x in _good_rows() if x["step"] == "G0" and x["arm"] == "u" and x["seed"] == "339102"]
    assert pair_state(ok_pair, no_stderr) == "OK" and pair_state(ok_pair[:2], no_stderr) == "MISSING"
    # (g) the rule is for G0 only and never reads hit facts: a hit-carrying skid run is replaced like any other (pf is a fact, not an input)
    rows_j, se_j = _skid_case({("u", "339102", "1")}, pf=3)
    assert se_of(se_j)(rows_j)["g0"] == ("PASS", "G0")
    # (h) the label of the code
    allp = {s: ("PASS", s.upper()) for s in STEPS}
    allp["g1"] = ("PASS", "G1")
    assert chain_code(None, {**allp, "g0": ("PASS", "G0T-C1-HOURS")}, None) == "G-PASS-C1-HOURS" and chain_code(None, allp, None) == "G-PASS"
    assert chain_code(None, {**allp, "g0": ("PASS", "G0T-C1-HOURS"), "g1": ("PASS", "G1-WITH-NONCOMPLETE")}, None) == "G-PASS-WITH-NONCOMPLETE-C1-HOURS"
    assert chain_code(None, {**allp, "g0": ("STOP", "G0T-KILL")}, None) == "G-STOP/G0T-KILL" and chain_code(None, {**allp, "g0": ("PASS", "G0T-C1-HOURS"), "g1": ("STOP", "G1-PU-LOW")}, None) == "G-STOP/G1-PU-LOW"
    # G0t for P
    # G0t (P): the P-u median at most 4.0 s; over: Qp 100 -> 50 once; still over after that: stop
    assert ok(with_(lambda x: x["step"] == "G0P", ms=4000))["g0p"] == ("PASS", "G0P")
    assert ok(with_(lambda x: x["step"] == "G0P", ms=4001))["g0p"] == ("STOP", "G0T-LOWER-QP")
    assert step_results(with_(lambda x: x["step"] == "G0P", ms=4000), (1000, 50, 339101))["g0p"] == ("PASS", "G0P")
    assert step_results(with_(lambda x: x["step"] == "G0P", ms=4001), (1000, 50, 339101))["g0p"] == ("STOP", "G0T-QP-SLOW-AFTER-LOWERING")
    assert ok(with_(lambda x: x["step"] == "G0P", ms=10001))["g0p"] == ("STOP", "G0T-LOWER-QP")
    assert ok(with_(lambda x: x["step"] == "G0P" and x["seed"] == "339101", rc=124))["g0p"] == ("STOP", "G0P-INCOMPLETE")
    # the projected-time check before G1: 1,000 x (P-u median) + 20 x (A-u median) + 600 s against the time left
    ra = _good_rows()                                    # P-u median 1.0 s, A-u median 1.0 s: need 1000 + 20 + 600 = 1620 s
    assert projected_pause(ra, 339101, 0, 1621) == (False, 1620.0, 1621)
    assert projected_pause(ra, 339101, 0, 1620)[0] is False and projected_pause(ra, 339101, 0, 1619)[0] is True
    assert projected_pause(ra, 339101, 4980, 6600)[0] is False and projected_pause(ra, 339101, 4981, 6600)[0] is True    # the time already used counts
    slowp = with_(lambda x: x["step"] == "G0P", ms=4000)     # a P-u median of 4 s: 4000 + 20 + 600 = 4620 s
    assert projected_pause(slowp, 339101, 0, 4620)[0] is False and projected_pause(slowp, 339101, 0, 4619)[0] is True
    slowa = with_(lambda x: x["step"] == "G0" and x["arm"] == "u", ms=1800)                 # an A-u median of 1.8 s adds 36 s
    assert abs(projected_pause(slowa, 339101, 0, 10000)[1] - 1636.0) < 1e-9
    assert projected_pause([x for x in ra if x["step"] != "G0P"], 339101, 0, 10**9)[0] is True      # no medians: stop, not a blind start
    # G1
    assert ok(_good_rows(uhits=3))["g1"] == ("PASS", "G1") and ok(_good_rows(uhits=2))["g1"] == ("STOP", "G1-PU-LOW")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "cp" and x["seed"] == "339250", pf=1))["g1"] == ("STOP", "G1-CONTROL-HIT")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "c" and x["seed"] == "339250", fnext=1))["g1"] == ("STOP", "G1-CONTROL-MECHANISM")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "c" and x["seed"] == "339250", handt=1))["g1"] == ("STOP", "G1-CONTROL-MECHANISM")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "u" and x["seed"] == "339399", rc=3, hf=1))["g1"] == ("STOP", "G1-HARNESS")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "u" and x["seed"] == "339399", rc=124))["g1"] == ("PASS", "G1-WITH-NONCOMPLETE")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "c" and x["seed"] == "339399", rc=125, hinfra=1, fnext=3))["g1"] == ("PASS", "G1-WITH-NONCOMPLETE")   # not rated, so its mechanism fact is not read
    assert ok([x for x in rows if not (x["step"] == "G1" and x["seed"] == "339400")])["g1"] == ("MISSING", "G1")
    # G2c
    assert ok(_good_rows(uhits=2))["g2c"] == ("STOP", "G2C-PU-LOW")
    assert ok(with_(lambda x: x["step"] == "G2C" and x["arm"] == "cp" and x["seed"] == "339300", vprop=1))["g2c"] == ("STOP", "G2C-CONTROL-HIT")
    assert ok(with_(lambda x: x["step"] == "G2C" and x["arm"] == "u" and x["seed"] == "339300", vharn=1))["g2c"] == ("STOP", "G2C-HARNESS")
    assert ok([x for x in rows if x["step"] != "G2C"])["g2c"] == ("MISSING", "G2C")
    assert ok(with_(lambda x: x["step"] == "G2C" and x["arm"] == "cp", rc=124))["g2c"] == ("STOP", "G2C-NO-RATED")
    assert ok(with_(lambda x: x["step"] == "G1" and x["arm"] == "c", rc=124))["g1"] == ("STOP", "G1-NO-RATED")
    # G2a is recorded: a missing seed is MISSING, nothing else changes the gates
    assert ok([x for x in rows if not (x["step"] == "G2A" and x["seed"] == "339405")])["g2a"][0] == "MISSING"
    assert ok(with_(lambda x: x["step"] == "G2A" and x["build"] == "v", fp="same", syscalls="117"))["g1"] == ("PASS", "G1")
    # the chain: exhaustive over the five step results
    n = 0
    kinds = (("PASS", "x"), ("STOP", "y"), ("MISSING", "z"))
    import itertools
    for combo in itertools.product(kinds, repeat=5):
        res = {s: (k[0], "G1" if s == "g1" and k[0] == "PASS" else k[1]) for s, k in zip(STEPS, combo)}
        for gate in (None, "SUDO"):
            for halt in (None, "BUDGET"):
                for bc in (False, True):
                  for pz in (False, True):
                    v = chain_code(gate, res, halt, bc, pz)
                    n += 1
                    assert v.startswith("G-")
                    allpass = gate is None and not bc and not pz and all(k[0] == "PASS" for k in combo)
                    assert v.startswith("G-PASS") == allpass, (combo, gate, bc, pz, v)
                    if gate:
                        assert v == "G-NOT-STARTED/SUDO"
                    elif bc:
                        assert v == "G-INVALID/BIN-CHANGED"
                    else:
                        first = next((s for s, k in zip(STEPS, combo) if k[0] != "PASS"), None)
                        if pz and first in (None, "g1", "g2a", "g2c"):
                            assert v == "G-PAUSE/BUDGET", (combo, v)        # the pause is read after g0 and g0p and before g1
                        elif first:
                            want = ("G-STOP/" + res[first][1]) if res[first][0] == "STOP" else "G-INCONCLUSIVE/%s-%s" % (FIRST[first], halt or "NOT-RUN")
                            assert v == want, (combo, v, want)
    pr = {s: ("PASS", s.upper()) for s in STEPS}
    pr["g1"] = ("PASS", "G1-WITH-NONCOMPLETE")
    assert chain_code(None, pr, None) == "G-PASS-WITH-NONCOMPLETE"
    g, h, a, b, p = steps_facts("1\tGATE-FAIL SUDO\n2\tARGS qw=500 qp=100 g0s=339511\n3\tBUDGET: past the no-new-work line\n4\tBIN-CHANGED u-shim\n5\tPAUSE-BUDGET need=1\n")
    assert g == "SUDO" and h == "BUDGET" and a == (500, 100, 339511) and b and p
    assert steps_facts("1\tGATE-READ g0p PASS G0P\n")[4] is False
    print("selftest ok: %d chain combinations" % n)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "g0first":
        try:
            print("ALLBAD" if g0_first_allbad(sclib.parse_rows(read_text(sys.argv[2])), int(sys.argv[3])) else "OK")
        except Exception as e:
            print("OK-ERROR/%s" % type(e).__name__)
    elif mode == "skidnext":
        try:
            pend = g0_effective(sclib.parse_rows(read_text(sys.argv[2])), int(sys.argv[5]), stderr_reader(out_dir_of(sys.argv[2])))[2]
            print("RUN %s" % pend[(sys.argv[3], sys.argv[4])] if (sys.argv[3], sys.argv[4]) in pend else "DONE")
        except Exception as e:
            print("DONE-ERROR/%s" % type(e).__name__)
    elif mode == "pause":
        try:
            pz, need, left = projected_pause(sclib.parse_rows(read_text(sys.argv[2])), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), stderr_reader(out_dir_of(sys.argv[2])))
            print("%s need=%s left=%s" % ("PAUSE" if pz else "OK", "NA" if need is None else "%.0f" % need, left))
        except Exception as e:
            print("PAUSE ERROR/%s" % type(e).__name__)
    elif mode == "gate":
        try:
            step, path, qw, qp, g0s = sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6])
            print(gate_text(step, sclib.parse_rows(read_text(path)), (qw, qp, g0s), stderr_reader(out_dir_of(path))))
        except Exception as e:
            print("STOP GATE-ERROR/%s" % type(e).__name__)
    elif mode == "verdict":
        main_verdict(sys.argv[2])
    elif mode == "selftest":
        selftest()
    else:
        sys.exit("usage: g_eval.py gate STEP RUNS_TSV QW QP G0S | g0first RUNS_TSV G0S | skidnext RUNS_TSV ARM SEED G0S | pause RUNS_TSV G0S NOW NEWBY | verdict RUN_DIR | selftest")
