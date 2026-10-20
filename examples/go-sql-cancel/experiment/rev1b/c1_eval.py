#!/usr/bin/env python3
# c1_eval.py : the pre-registered rules of the C1 spec (GOC1 rev 1). Nothing here is tuned after seeing data. It imports the GOG rev 3a g_eval.py (unchanged: the class
# function, the SKID_OVERSHOOT signature, the G0 effective-rows reader) and sclib.py (unchanged: classes, Wilson, exact zero bound, median).
#   c1_eval.py next RUNS_TSV                        cells to run next: "RUN C1 <arm> <seed> <idx>" lines (one block), or "DONE", or "ERROR <why>"   (c1.sh, between blocks)
#   c1_eval.py halts RUNS_TSV                       "OK" or "HALT <code>": the infrastructure halts over THIS window's rows (c1.sh, after every block). No result stops a window.
#   c1_eval.py probe BLOCK_S NBLOCKS ELAPSED        the rate check after the first NBLOCKS blocks (ELAPSED wall seconds): "OK b=<s> plan=<s>" or "PAUSE-RATE ..." (c1.sh)
#   c1_eval.py pausehours PREV_DIR BLOCK_S CAP_S MAXBLOCKS   the projected-hours check at a window start: "OK ..." or "PAUSE-HOURS ..." (c1.sh)
#   c1_eval.py prevcheck PREV_DIR [QW]              "OK ..." or "BAD <why>": the previous-windows bundle is intact and every earlier window ran at QW (c1.sh and c2e.sh gate)
#   c1_eval.py hits PREV_DIR                        the A-u hit seeds that C2 confirms: "HITS <total> <listed>" then one seed per line, or "NONE" (c1.sh confirm, c2e.sh)
#   c1_eval.py plan G0_RUNS_TSV G0S [HARD_S]        the mechanical window plan from a GOG launch's G0 timing (Mac, before the launch sheet is fixed)
#   c1_eval.py prepare OUT_DIR WINDOW_DIR...        builds the prev bundle for the next window from the archived windows (Mac)
#   c1_eval.py verdict --g1 PASS|NOTPASS [--skid-b] WINDOW_DIR...   prints the tables and "OUTCOME <code>" (Mac, over the archive copies, windows in order). Rev 1b: with --skid-b
#       it also prints "OUTCOME-SKID-B <code>", the code with 'shape B' Hermit failures read as the skid ledger (5.6, 8), per run, only in a window whose skid-b-leftover.txt
#       first line is exactly CLEAN; the mechanical OUTCOME line is printed first and is the same with and without the flag. Only this mode and analyse() changed in rev 1b.
#   c1_eval.py selftest                             exhaustive and sample tests, no files written (also run on the box as gate EVAL-SELFTEST)
# Test overrides (Mac harness only; gate OVERRIDES refuses any C_* variable on the box unless C_TEST=1): C_TEST=1 with C_NIDX.
import hashlib
import json
import math
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import g_eval  # noqa: E402
import sclib  # noqa: E402

ARMS = ("u", "c", "cp")                      # A-u go1.24.5, A-c go1.24.6, A-c' go1.24.5 plus the fix (design 6.2)
CONTROLS = ("c", "cp")
SEED0 = 330000                               # counted index i (1..2000) runs with --sched-seed 330000+i in every arm (design 7: 330001-332000, one identical list)
CFG = {"n": 2000}                            # 2,000 seeds per arm (design 2.5, 7.2)
if os.environ.get("C_TEST") == "1":
    CFG["n"] = int(os.environ.get("C_NIDX", CFG["n"]))
TIMEOUT_HALT = 20                            # a window with this many TIMEOUT runs halts (infrastructure: the box is too slow; a choice, spec 5.5)
HERMIT_FAIL_HALT = 40                        # a window with this many Hermit failures halts (the environment is not the one measured; a choice, spec 5.5)
ALLBAD_RUNS = 15                             # a window whose first 15 counted runs (5 blocks) all failed to complete halts (F0 and GOG ALL-BAD precedent)
C2_MAX = 100                                 # at most this many A-u hits are confirmed by C2 (spec 5.9; a choice)
PAUSE_FACTOR = 2.0                           # the probe pauses when the observed block time is more than this multiple of the plan (spec 5.6; a choice)
WIN_HARD_S, SOFT_GUARD_S = 6600, 180         # GOG's no-new-run line (110 min) and the block guard (3 runs x the 60 s cap)
SETUP_S, MARGIN_S, SPARE_RESERVE_RUNS = 120, 300, 10
OVERHEAD_S = SETUP_S + MARGIN_S              # per window, in the hours check
SIZING_PCT = 95                              # the plan sizes a window at 95% of the time that is available
DRIVER_S_PER_RUN = 0.25                      # the driver's own seconds per run on top of Hermit's wall (python, sha256sum, facts, guard scans); GOG launch 1 measured about 0.16 (a choice with margin)
NATIVE_HIGH = False                          # R-HIT-NATIVE-EASY: GON's N-u lower bound 3.4965% is below the 5% rule, so the code never applies (design 4.3, 8.3)
G1_FIELD = "PASS"
REPLAY_LINE = "output matches the recording: yes; schedule matches the recording: yes; the recorded property failed again: yes"


def cls(r):
    return g_eval.cls(r)


def iseed(r):
    try:
        return int(r["seed"])
    except (ValueError, TypeError):
        return -1


def rotation(idx):
    """The arm order of block idx: the starting arm rotates with the index, as in N (so every arm has the same count and the same positions)."""
    s = (idx - 1) % 3
    return [ARMS[(s + j) % 3] for j in range(3)]


def malformed(text):
    """Line numbers of runs.tsv lines that do not have the schema's column count (a kill between the two writes of one row would leave one)."""
    return [i + 1 for i, ln in enumerate(text.splitlines()) if ln.strip() and len(ln.split("\t")) != len(sclib.COLS)]


def multi_reader(runs_tsv):
    base = os.path.dirname(os.path.abspath(runs_tsv))
    dirs = [os.path.join(base, "out"), os.path.join(base, "prev", "out")]

    def se(run_id):
        for d in dirs:
            t = g_eval.read_text(os.path.join(d, run_id + ".stderr"))
            if t:
                return t
        return ""
    return se


def load_text_rows(runs_tsv):
    base = os.path.dirname(os.path.abspath(runs_tsv))
    return g_eval.read_text(os.path.join(base, "prev", "runs.tsv")), g_eval.read_text(runs_tsv)


def skid(r, se):
    return g_eval.skid_run(r, se(r["id"]))


# ---------------------------------------------------------------- 'shape B' (rev 1b; verdict side only, never read by the box-side modes)
# Rule fixed 2026-10-18T01:17Z, after window 1: the same trigger as the SKID_OVERSHOOT signature of GOG 5.3a up to the panic, but the panic is followed by Hermit's cleanup-unconfirmed
# path instead of a task exit. Selection rule, never a decision run by run: exit 125; the HERMIT_SKID_OVERSHOOT rcb_actual line; the panic at detcore/src/lib.rs:348 with the
# HERMIT_SKID_OVERSHOOT prehook message on ANY thread; then exactly HERMIT_CLEANUP_UNCONFIRMED ... Poisoned, HERMIT_INTERNAL_FAILURE class=container-child-panic and the
# 'Hermit cleanup remains unconfirmed (Poisoned' error line; and no other Hermit marker. It is read as the skid ledger FOR CLAIMS only when the window's leftover scan was clean.
SKIDB_PANIC_RE = re.compile(r"^thread '[^']+' \(\d+\) panicked at detcore/src/lib\.rs:348:\d+:$")
SKIDB_PREHOOK = "HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot!"
SKIDB_CLEANUP_RE = re.compile(r"^HERMIT_CLEANUP_UNCONFIRMED: backend phase Some\(Poisoned\); namespace retirement observed=true(;|$)")
SKIDB_INTERNAL = "HERMIT_INTERNAL_FAILURE class=container-child-panic"
SKIDB_ERROR = "Error: Hermit cleanup remains unconfirmed (Poisoned"
SKIDB_SIGNAL = "WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL "
LEFTOVER_FILE = "skid-b-leftover.txt"


def skid_b_run(r, text):
    """True when the run is a Hermit failure of shape B (strict). Exit status 125 and no Go fatal or deadlock fact; each of the six lines below exactly once; every other line that
    carries a Hermit marker, a 'panicked at', an 'Error:' or a Hermit usage or deadlock text makes it False."""
    if r["rc"] != 125 or r["fatal"] > 0 or r["dead"] > 0:
        return False
    n = {"skid": 0, "panic": 0, "prehook": 0, "cleanup": 0, "internal": 0, "error": 0}
    for ln in text.splitlines():
        if g_eval.SKID_LINE.match(ln):
            n["skid"] += 1
        elif SKIDB_PANIC_RE.match(ln):
            n["panic"] += 1
        elif ln.startswith(SKIDB_PREHOOK):
            n["prehook"] += 1
        elif SKIDB_CLEANUP_RE.match(ln):
            n["cleanup"] += 1
        elif ln == SKIDB_INTERNAL:
            n["internal"] += 1
        elif ln.startswith(SKIDB_ERROR):
            n["error"] += 1
        elif SKIDB_SIGNAL in ln:
            continue
        elif "HERMIT" in ln or "panicked at" in ln or ln.startswith("Error:") or "Deadlock detected" in ln or "Usage: hermit" in ln or "tool_global.rs" in ln:
            return False
    return all(v == 1 for v in n.values())


def leftover_clean(text):
    """The window's leftover scan was clean: the first line of skid-b-leftover.txt is exactly CLEAN (written from the section 11 reads after the window: no Hermit process, mount,
    cgroup or /dev/shm/hairspring-* entry). An absent or empty file is not clean."""
    return text.splitlines()[:1] == ["CLEAN"]


# ---------------------------------------------------------------- cells, integrity, what to run next
def cells(rows):
    d = {}
    for r in rows:
        if r["step"] == "C1":
            d.setdefault((r["arm"], iseed(r)), []).append(r)
    return d


def integrity(rows):
    errs, seen = [], set()
    for r in rows:
        if r["step"] not in ("C1", "C2D"):
            errs.append("unknown step %s in %s" % (r["step"], r["id"]))
            continue
        if r["arm"] not in ARMS or r["build"] != "s" or r["mode"] != "std":
            errs.append("%s: arm, build or mode outside C1 (%s %s %s)" % (r["id"], r["arm"], r["build"], r["mode"]))
        key = (r["step"], r["arm"], r["seed"], r["rep"])
        if key in seen:
            errs.append("duplicate cell %s" % (key,))
        seen.add(key)
        s = iseed(r)
        if r["step"] == "C1" and not (SEED0 + 1 <= s <= SEED0 + CFG["n"]):
            errs.append("%s: seed %s outside the counted block" % (r["id"], r["seed"]))
        if r["step"] == "C1" and r["rep"] != "1":
            errs.append("%s: rep %s in a counted cell" % (r["id"], r["rep"]))
        if r["step"] == "C2D" and (r["arm"] != "u" or r["rep"] not in ("1", "2", "3")):
            errs.append("%s: a C2D row must be A-u rep 1 to 3" % r["id"])
    return errs


def missing_idx(rows):
    """For each arm, the counted indices that have no row (any class: a Hermit failure, a timeout or a crash is a decided cell and is never run again)."""
    have = {a: set() for a in ARMS}
    for r in rows:
        if r["step"] == "C1" and r["arm"] in have:
            have[r["arm"]].add(iseed(r) - SEED0)
    return {a: [i for i in range(1, CFG["n"] + 1) if i not in have[a]] for a in ARMS}


def next_cells(rows, bad_lines=()):
    if bad_lines:
        return ["ERROR malformed runs.tsv line %s" % bad_lines[0]]
    errs = integrity(rows)
    if errs:
        return ["ERROR " + errs[0]]
    miss = missing_idx(rows)
    firsts = [m[0] for m in miss.values() if m]
    if not firsts:
        return ["DONE"]
    idx = min(firsts)
    return ["RUN C1 %s %d %d" % (a, SEED0 + idx, idx) for a in rotation(idx) if idx in miss[a]]


# ---------------------------------------------------------------- infrastructure halts (this window's rows); no result (a hit, a control hit, a harness failure, a crash) stops a window
def halts(rows):
    c1 = [r for r in rows if r["step"] == "C1"]
    if len(c1) >= ALLBAD_RUNS and all(cls(r) not in sclib.RATED for r in c1[:ALLBAD_RUNS]):
        return "C1-HALT/ALL-BAD"
    cs = [cls(r) for r in rows if r["step"] in ("C1", "C2D")]
    if cs.count("HERMIT-FAIL") >= HERMIT_FAIL_HALT:
        return "C1-HALT/HERMIT-FAIL-RATE"
    if cs.count("TIMEOUT") >= TIMEOUT_HALT:
        return "C1-HALT/TIMEOUT-RATE"
    return None


# ---------------------------------------------------------------- projected-time checks and the mechanical plan
def blocks_decided(rows):
    cl = cells(rows)
    return sum(1 for idx in range(1, CFG["n"] + 1) if all((a, SEED0 + idx) in cl for a in ARMS))


def pause_hours(prev_rows, walls, block_s, cap_s, maxblocks):
    """-> (pause, used_s, need_s). need = the undecided blocks x BLOCK_S + OVERHEAD_S per remaining window; used = the wall seconds of the earlier windows."""
    todo = CFG["n"] - blocks_decided(prev_rows)
    wins = -(-todo // max(1, maxblocks))
    need = todo * block_s + wins * OVERHEAD_S
    used = sum(walls)
    return used + need > cap_s, used, need


def probe(block_s, nblocks, elapsed):
    """After the first NBLOCKS blocks of the window: b = observed wall seconds per block (the driver's own clock, its overhead included). A block more than PAUSE_FACTOR
    times slower than the plan's BLOCK_S pauses the window. -> (verdict, b)."""
    if nblocks <= 0:
        return "OK", None
    b = elapsed / float(nblocks)
    return ("PAUSE-RATE" if b > PAUSE_FACTOR * block_s else "OK"), b


def plan(g0_rows, g0s, se=g_eval.no_stderr, hard_s=WIN_HARD_S):
    """The mechanical window plan from a GOG launch's effective G0 rows (timing only: the wall milliseconds of the rated runs). -> dict, or None when an arm has no rated run."""
    eff = g_eval.effective_rows(g0_rows, g0s, se)
    ms = {}
    for a in ARMS:
        xs = [r["ms"] for r in eff if r["arm"] == a and cls(r) in sclib.RATED]
        if not xs:
            return None
        ms[a] = sum(xs) / float(len(xs))
    block_s = int(math.ceil(sum(ms.values()) / 1000.0 + 3 * DRIVER_S_PER_RUN))
    pmax = max(ms.values()) / 1000.0 + DRIVER_S_PER_RUN
    avail = hard_s - SOFT_GUARD_S - (SETUP_S + 2 * pmax) - MARGIN_S - SPARE_RESERVE_RUNS * pmax
    sized = int(avail * SIZING_PCT / 100.0)
    bpw_max = max(1, sized // block_s)
    nwin = -(-CFG["n"] // bpw_max)
    bpw = -(-CFG["n"] // nwin)          # balanced windows
    run_h = CFG["n"] * (sum(ms.values()) / 1000.0 + 3 * DRIVER_S_PER_RUN) / 3600.0
    return {"mean_ms": ms, "block_s": block_s, "avail_s": avail, "bpw": bpw, "nwin": nwin, "run_hours": run_h,
            "total_hours": run_h + nwin * OVERHEAD_S / 3600.0,
            "ranges": [(k * bpw + 1, min(CFG["n"], (k + 1) * bpw)) for k in range(nwin)]}


# ---------------------------------------------------------------- statistics (design 8): R-HIT
def r_hit(k_u, n_u, n_c, n_cp):
    """Design 8.3: both counted controls at 0 and the A-u Wilson lower bound exceeds the larger Wilson upper bound of A-c and A-c' (k >= 8 at N = 2,000 each)."""
    lo, _ = sclib.wilson(k_u, n_u)
    his = [sclib.wilson(0, n)[1] for n in (n_c, n_cp)]
    return lo is not None and None not in his and lo > max(his)


# ---------------------------------------------------------------- the hit witness and the confirmations (E-conditions)
FF_RE = re.compile(r'"first_false":\{"thread":"[^"]*","ord": *(\d+),"vals":\[(-?\d+),(-?\d+),(-?\d+),(-?\d+)\]')
SEED_RE = re.compile(r"sched-seed (\d+)")


def witness_class(stderr_text):
    """The class (1 foreign, 2 torn, 3 unparsable, 0 own) of the first false evaluation of the property site, or None."""
    for line in stderr_text.splitlines():
        if '"kind":"assert"' in line and '"id":"%s"' % sclib.PROP_ID in line:
            m = FF_RE.search(line)
            return int(m.group(5)) if m else None
    return None


def confirmation(hit, c2d_rows):
    """C2 part a, 3 reruns of the hit's own seed on the same binary by the direct driver: each must be class HIT with the original run's stdout sha and fingerprint.
    -> (n_confirming, n_reruns)."""
    mine = [r for r in c2d_rows if r["arm"] == "u" and r["seed"] == hit["seed"]]
    ok = [r for r in mine if cls(r) == "HIT" and r["stdout_sha"] == hit["stdout_sha"] and r["fp"] == hit["fp"]]
    return len(ok), len(mine)


def engine_result(test_text, replay_text, seed):
    """C2 part b, the engine's records for one hit seed. test_text: the JSON of `hairspring test`; replay_text: the text of `hairspring replay` ('' when no replay was made).
    -> (eng, rep): None when there is no test record at all; else True or False. eng: status failed, the failing sched-seed is the hit's own seed, and 'reproduced 3/3'.
    rep: the replay line says output, schedule and property all match."""
    if not test_text.strip():
        return None, None
    try:
        o = json.loads(test_text)[0]["outcome"]
        msg = (o.get("failure") or {}).get("message", "")
        m = SEED_RE.search(msg)
        eng = o.get("status") == "failed" and m is not None and int(m.group(1)) == seed and "reproduced 3/3" in msg
    except (ValueError, KeyError, IndexError, TypeError, AttributeError):
        eng = False
    return bool(eng), REPLAY_LINE in replay_text


# ---------------------------------------------------------------- analysis and the outcome code
def analyse(rows, se, engine=None, skid_b=None):
    """engine: seed -> (eng, rep) from engine_result (absent seeds: no record). skid_b (rev 1b, verdict only): None, or a function row -> bool that says a Hermit-failure row is
    a shape B failure in a clean window; such rows then count as the skid ledger. With None the behaviour is that of rev 1a."""
    engine = engine or {}
    cr = [r for r in rows if r["step"] == "C1"]
    an = {"integrity": integrity(rows), "arms": {}, "hits": [], "complete": True}
    miss = missing_idx(rows)
    for a in ARMS:
        mine = [r for r in cr if r["arm"] == a]
        cs = [cls(r) for r in mine]
        skids = [r for r, c in zip(mine, cs) if c == "HERMIT-FAIL" and (skid(r, se) or (skid_b is not None and skid_b(r)))]
        d = {"rated": sum(1 for c in cs if c in sclib.RATED), "hit": sum(1 for c in cs if c == "HIT"),
             "timeout": cs.count("TIMEOUT"), "crash": cs.count("CRASH"), "harness": cs.count("HARNESS-FAIL"),
             "hermit": cs.count("HERMIT-FAIL"), "hermit_skid": len(skids), "hermit_other": cs.count("HERMIT-FAIL") - len(skids),
             "hit_detail_all": sum(1 for r in mine if sclib.hit_detail(r)), "decided": len(mine), "missing": len(miss[a])}
        d["complete"] = not miss[a]
        an["complete"] = an["complete"] and d["complete"]
        an["arms"][a] = d
    c2d = [r for r in rows if r["step"] == "C2D"]
    # C2 confirms the first m = min(k, C2_MAX) A-u hit seeds in seed order (deterministic, nothing chosen); the E-conditions are judged on those m, while k and the
    # rate come from all the counted runs (arms["u"]["hit"]).
    all_hits = sorted((r for r in cr if r["arm"] == "u" and cls(r) == "HIT"), key=iseed)
    an["k_total"], an["m"] = len(all_hits), min(len(all_hits), C2_MAX)
    for r in all_hits[:C2_MAX]:
        wc = witness_class(se(r["id"]))
        conf, n = confirmation(r, c2d)
        eng, rep = engine.get(iseed(r), (None, None))
        an["hits"].append({"id": r["id"], "seed": r["seed"], "class": wc, "confirmed": conf, "reruns": n,
                           "ok": conf == 3 and n == 3 and wc in (1, 2), "eng": eng, "rep": rep})
    return an


def outcome(an, gate=None, bin_changed=False, g1=None, any_cells=True):
    """The first rule that holds (spec 8). -> code. g1: 'PASS' when GOG's G1 passed (a launch-sheet fact). Design codes after the state prefix; the words C2-PENDING (C2 not yet
    run), E-UNMET/DIRECT, E-UNMET/ENGINE, R-HIT-CANDIDATE and R-INCONC-NO-G1 are this spec's, for the steps between the counted runs and the existence claim. an["hits"] holds
    the m = min(k, C2_MAX) sampled hits; k = arms["u"]["hit"] counts all of them."""
    if gate and not any_cells:
        return "C1-NOT-STARTED/%s" % gate
    if bin_changed:
        return "C1-INVALID/BIN-CHANGED"
    if an["integrity"]:
        return "C1-INVALID/INTEGRITY"
    arms = an["arms"]
    state = "DONE" if an["complete"] else "PARTIAL"
    suf = ""
    hs = {a: arms[a]["hermit_skid"] for a in ARMS}
    if any(hs.values()):
        suf += "+R-CRASH-hermit(u=%d,c=%d,cp=%d)" % (hs["u"], hs["c"], hs["cp"])
    if any(arms[a]["timeout"] for a in ARMS):
        suf += "+TIMEOUTS(u=%d,c=%d,cp=%d)" % tuple(arms[a]["timeout"] for a in ARMS)
    if any(arms[a]["hit_detail_all"] for a in CONTROLS):
        return "C1-%s/R-CONTROL-HIT%s" % (state, suf)
    if any(arms[a]["harness"] for a in ARMS):
        return "C1-%s/R-HARNESS%s" % (state, suf)
    if any(arms[a]["crash"] or arms[a]["hermit_other"] for a in ARMS):
        return "C1-%s/R-CRASH%s" % (state, suf)
    if not an["complete"]:
        return "C1-PARTIAL/R-INCONC%s" % suf
    if g1 != G1_FIELD:
        return "C1-DONE/R-INCONC-NO-G1%s" % suf
    k = arms["u"]["hit"]
    if k == 0:
        return "C1-DONE/R-NULL%s" % suf
    hits = an["hits"]
    if any(h.get("reruns", 1) == 0 for h in hits):
        return "C1-DONE/C2-PENDING%s" % suf
    if any(not h["ok"] for h in hits):
        return "C1-DONE/E-UNMET/DIRECT%s" % suf
    if any(h.get("eng") is False or h.get("rep") is False for h in hits):
        return "C1-DONE/E-UNMET/ENGINE%s" % suf
    if any(h.get("eng") is None or h.get("rep") is None for h in hits):
        return "C1-DONE/R-HIT-CANDIDATE%s" % suf
    if r_hit(k, arms["u"]["rated"], arms["c"]["rated"], arms["cp"]["rated"]):
        return "C1-DONE/R-HIT-NATIVE-EASY%s" % suf if NATIVE_HIGH else "C1-DONE/R-HIT%s" % suf
    return "C1-DONE/R-HIT-FEW%s" % suf


# ---------------------------------------------------------------- steps.tsv facts
INFRA_KEYS = ("BUDGET", "HALTED", "SIGNAL", "IDLE-EXHAUSTED", "REFUSED", "SIZE-CAP", "STOPPED", "TIMEOUT-HALT")


def window_meta(steps_text):
    m = {"gate": None, "args": None, "halt_rule": None, "pause": None, "bin_changed": False, "infra": None, "full": False, "complete": False, "c2d": False, "c2e": False,
         "wall": None, "t_first": None, "t_last": None}
    for line in steps_text.splitlines():
        first, n = (line.split("\t", 1) + [""])[:2] if "\t" in line else ("", line)
        if first.isdigit():
            m["t_first"] = int(first) if m["t_first"] is None else m["t_first"]
            m["t_last"] = int(first)
        g = re.match(r"GATE-FAIL (\S+)", n)
        if g:
            m["gate"] = g.group(1)
        a = re.match(r"ARGS (.*)", n)
        if a:
            m["args"] = a.group(1)
        h = re.match(r"HALT-RULE (\S+)", n)
        if h and m["halt_rule"] is None:
            m["halt_rule"] = h.group(1)
        p = re.match(r"PAUSE-BUDGET (\S+)", n)
        if p and m["pause"] is None:
            m["pause"] = p.group(1)
        if n.startswith("BIN-CHANGED"):
            m["bin_changed"] = True
        if n.startswith("WINDOW-FULL"):
            m["full"] = True
        if n.startswith("C1-COMPLETE"):
            m["complete"] = True
        if n.startswith("C2D-COMPLETE"):
            m["c2d"] = True
        if n.startswith("C2E-COMPLETE"):
            m["c2e"] = True
        w = re.match(r"WINDOW-WALL (\d+)", n)
        if w:
            m["wall"] = int(w.group(1))
        for key in INFRA_KEYS:
            if n.startswith(key) and m["infra"] is None:
                m["infra"] = key
    if m["wall"] is None and m["t_first"] is not None:
        m["wall"] = m["t_last"] - m["t_first"]
    return m


def window_end(m):
    return m["gate"] and "GATE-FAIL/" + m["gate"] or m["halt_rule"] or m["pause"] or m["infra"] or (
        "COMPLETE" if m["complete"] else "C2D-COMPLETE" if m["c2d"] else "C2E-COMPLETE" if m["c2e"] else "FULL" if m["full"] else "NO-END-LINE")


# ---------------------------------------------------------------- the previous-windows bundle
def sha_of(text):
    return hashlib.sha256(text.encode()).hexdigest()


def prev_ok(runs_text, walls_text, sha_text, qw=None):
    """prev.sha holds two lines '<sha256> runs.tsv' and '<sha256> walls.tsv'; the contents must match, parse, and carry no malformed line; with qw given, every earlier
    window ran at that Qw (the -q of a counted run is the same in every window)."""
    want = dict((ln.split()[1], ln.split()[0]) for ln in sha_text.splitlines() if len(ln.split()) == 2)
    if want.get("runs.tsv") != sha_of(runs_text) or want.get("walls.tsv") != sha_of(walls_text):
        return "BAD sha256 mismatch"
    if qw is not None and any(q != str(qw) for q in qws_of(walls_text)):
        return "BAD qw mismatch: earlier windows ran at %s, this one at %s" % (sorted(set(qws_of(walls_text))), qw)
    if malformed(runs_text):
        return "BAD malformed line %d" % malformed(runs_text)[0]
    rows = sclib.parse_rows(runs_text)
    errs = integrity(rows)
    if errs:
        return "BAD " + errs[0]
    return "OK rows=%d blocks=%d" % (len(rows), blocks_decided(rows))


def walls_of(text):
    """walls.tsv lines: window number, wall seconds, qw."""
    out = []
    for ln in text.splitlines():
        f = ln.split("\t")
        if len(f) == 3 and f[1].isdigit():
            out.append(int(f[1]))
    return out


def qws_of(text):
    return [ln.split("\t")[2] for ln in text.splitlines() if len(ln.split("\t")) == 3]


def hit_seeds(rows):
    seeds = sorted(iseed(r) for r in rows if r["step"] == "C1" and r["arm"] == "u" and cls(r) == "HIT")
    return len(seeds), seeds[:C2_MAX]


# ---------------------------------------------------------------- CLI helpers
def cmd_prepare(out_dir, wdirs):
    os.makedirs(os.path.join(out_dir, "out"), exist_ok=True)
    runs, walls = "", ""
    for i, d in enumerate(wdirs, 1):
        t = g_eval.read_text(os.path.join(d, "runs.tsv"))
        runs += t if (t.endswith("\n") or not t) else t + "\n"
        meta = window_meta(g_eval.read_text(os.path.join(d, "steps.tsv")))
        qm = re.search(r"qw=(\d+)", meta["args"] or "")
        walls += "%d\t%d\t%s\n" % (i, meta["wall"] or 0, qm.group(1) if qm else "0")
        for r in sclib.parse_rows(t):
            if cls(r) == "HERMIT-FAIL":
                src = os.path.join(d, "out", r["id"] + ".stderr")
                with open(os.path.join(out_dir, "out", r["id"] + ".stderr"), "w") as f:
                    f.write(g_eval.read_text(src))
    for name, text in (("runs.tsv", runs), ("walls.tsv", walls)):
        with open(os.path.join(out_dir, name), "w") as f:
            f.write(text)
    with open(os.path.join(out_dir, "prev.sha"), "w") as f:
        f.write("%s runs.tsv\n%s walls.tsv\n" % (sha_of(runs), sha_of(walls)))
    print(prev_ok(runs, walls, "%s runs.tsv\n%s walls.tsv\n" % (sha_of(runs), sha_of(walls))))


def main_verdict(args):
    g1, dirs, i, want_b = None, [], 0, False
    while i < len(args):
        if args[i] == "--g1":
            g1 = args[i + 1]
            i += 2
        elif args[i] == "--skid-b":
            want_b = True
            i += 1
        else:
            dirs.append(args[i])
            i += 1
    rows, metas, readers, win_of = [], [], [], {}
    for d in dirs:
        for r in sclib.parse_rows(g_eval.read_text(os.path.join(d, "runs.tsv"))):
            win_of[r["id"]] = d
        rows += sclib.parse_rows(g_eval.read_text(os.path.join(d, "runs.tsv")))
        metas.append(window_meta(g_eval.read_text(os.path.join(d, "steps.tsv"))))
        readers.append(g_eval.stderr_reader(os.path.join(d, "out")))
        bad = malformed(g_eval.read_text(os.path.join(d, "runs.tsv")))
        if bad:
            print("MALFORMED runs.tsv line(s) in %s: %s" % (d, bad[:5]))

    def se(run_id):
        for rd in readers:
            t = rd(run_id)
            if t:
                return t
        return ""

    def out_text(name):
        for d in dirs:
            t = g_eval.read_text(os.path.join(d, "out", name))
            if t:
                return t
        return ""

    engine = {}
    for r in rows:
        if r["step"] == "C1" and r["arm"] == "u" and cls(r) == "HIT":
            s = iseed(r)
            engine[s] = engine_result(out_text("C2E.%d.t.out" % s), out_text("C2E.%d.r.out" % s), s)
    print("launch-sheet fact used (not read from the windows): --g1 %s (GOG's G1 passed)" % g1)
    print("windows (in order), the end line of each:")
    for i, (d, m) in enumerate(zip(dirs, metas), 1):
        idxs = [iseed(r) - SEED0 for r in sclib.parse_rows(g_eval.read_text(os.path.join(d, "runs.tsv"))) if r["step"] == "C1"]
        rng = "C1 indices %d-%d (%d runs)" % (min(idxs), max(idxs), len(idxs)) if idxs else "no C1 run"
        print("  window %d: %s | args: %s | %s | end: %s | wall %s s | binary changed: %s" % (i, os.path.basename(d.rstrip("/")), m["args"], rng, window_end(m), m["wall"], m["bin_changed"]))
    an = analyse(rows, se, engine)
    print("integrity errors:", an["integrity"] or "none")
    for a in ARMS:
        d = an["arms"][a]
        print("A-%-2s decided=%d rated=%d | HIT %s | not rated: timeout=%d crash=%d harness=%d hermit=%d (skid signature %d, other %d) | missing=%d" % (
            a, d["decided"], d["rated"], sclib.rate_text(d["hit"], d["rated"]), d["timeout"], d["crash"], d["harness"], d["hermit"], d["hermit_skid"], d["hermit_other"], d["missing"]))
        print("   R-CRASH ledger A-%s (Hermit failures, reason hermit, outside the rated runs, not replaced): %s" % (
            a, ", ".join(r["id"] for r in rows if r["step"] == "C1" and r["arm"] == a and cls(r) == "HERMIT-FAIL") or "none"))
        print("   sensitivity (information, not a rule): if every cell that is not rated had been a hit, the hit count would be at most %d of %d" % (
            d["hit"] + d["decided"] - d["rated"], d["decided"]))
    ctl = [r["id"] for r in rows if r["step"] == "C1" and r["arm"] in CONTROLS and sclib.hit_detail(r)]
    print("control hits (A-c, A-c'; in full if any):", ctl or "none")
    if an["k_total"]:
        print("the property site was false in k = %d of %d blind seeds (rated %d); the first m = %d of them in seed order are each confirmed (3/3 identical direct reruns, the engine at the same seed, bundle replay) or listed below" % (
            an["k_total"], CFG["n"], an["arms"]["u"]["rated"], an["m"]))
    for h in an["hits"]:
        print("A-u hit %s seed %s: witness class %s (1 foreign, 2 torn, 3 unparsable) | C2a direct reruns confirming %d of %d | C2b engine reproduced at the same seed %s | bundle replay matches %s | C2a ok: %s" % (
            h["id"], h["seed"], h["class"], h["confirmed"], h["reruns"], h["eng"], h["rep"], "yes" if h["ok"] else "NO"))
    c2d_rows = [r for r in rows if r["step"] == "C2D"]
    if c2d_rows:
        cc = {}
        for r in c2d_rows:
            cc[cls(r)] = cc.get(cls(r), 0) + 1
        print("C2a rows: %d | classes %s (a rerun that is not a HIT leaves its seed unconfirmed; it is not a blocker by itself)" % (len(c2d_rows), cc))
    u, c, cp = (an["arms"][a] for a in ARMS)
    print("R-HIT arithmetic (design 8.3): A-u Wilson lower %s vs larger control upper %s -> r_hit=%s; native 'high' (R-HIT-NATIVE-EASY) applies: %s" % (
        sclib.pct(sclib.wilson(u["hit"], u["rated"])[0]), sclib.pct(max(sclib.wilson(0, c["rated"])[1] or 0, sclib.wilson(0, cp["rated"])[1] or 0)),
        r_hit(u["hit"], u["rated"], c["rated"], cp["rated"]), NATIVE_HIGH))
    for a in ARMS:
        ms = [r["ms"] for r in rows if r["step"] == "C1" and r["arm"] == a]
        print("wall per run A-%s: median %s ms, mean %.0f ms over %d runs (context only)" % (a, sclib.median(ms), sum(ms) / max(1, len(ms)), len(ms)))
    last = metas[-1] if metas else None
    print("OUTCOME", outcome(an, gate=last["gate"] if last else "NO-WINDOWS", bin_changed=any(m["bin_changed"] for m in metas), g1=g1, any_cells=bool(rows)))
    if want_b:
        clean = {d: leftover_clean(g_eval.read_text(os.path.join(d, LEFTOVER_FILE))) for d in dirs}
        fb = [r for r in rows if r["step"] == "C1" and cls(r) == "HERMIT-FAIL" and not skid(r, se) and skid_b_run(r, se(r["id"]))]
        print("shape B (rule fixed 2026-10-18T01:17Z; one selection rule, never a decision run by run): %d Hermit-failure run(s) match the strict shape B signature" % len(fb))
        for r in fb:
            print("  %s (arm %s, window %s): leftover scan %s -> %s" % (r["id"], r["arm"], os.path.basename(win_of[r["id"]].rstrip("/")), "CLEAN" if clean[win_of[r["id"]]] else "NOT CLEAN or file missing",
                                                                      "read as the skid ledger" if clean[win_of[r["id"]]] else "stays a blocking R-CRASH"))
        an_b = analyse(rows, se, engine, lambda r: clean[win_of[r["id"]]] and skid_b_run(r, se(r["id"])))
        print("OUTCOME-SKID-B", outcome(an_b, gate=last["gate"] if last else "NO-WINDOWS", bin_changed=any(m["bin_changed"] for m in metas), g1=g1, any_cells=bool(rows)))
        print("(the mechanical OUTCOME above is unchanged by this flag; the mechanical code may be accepted as final instead)")


# ---------------------------------------------------------------- selftest
def _row(step, arm, seed, rep=1, **kw):
    args = {"build": "s", "mode": "std", "id": "%s.%s.%s.%s" % (step, arm, seed, rep)}
    args.update(kw)
    return g_eval._row(step, arm, seed, rep, **args)


def _crow(arm, idx, **kw):
    return _row("C1", arm, SEED0 + idx, **kw)


def _skid_kw():
    return dict(rc=125, hinfra=1, hpanic=1, fp="NOSUMMARY", syscalls="NA", ms=16000)


def _all_cells(n, skip=()):
    return [_crow(a, i) for i in range(1, n + 1) for a in ARMS if (a, i) not in skip]


def _oracle(gate, binchg, integ, complete, chit, harn, crash, hother, k, g1, okh, eng, n_rated):
    """A flat restatement of outcome() for the exhaustive test (arms all at n_rated rated runs, no skid failures, no timeouts). eng: 'yes' | 'none' | 'no' for the
    engine confirmation and the replay of every hit."""
    if binchg:
        return "C1-INVALID/BIN-CHANGED"
    if integ:
        return "C1-INVALID/INTEGRITY"
    state = "DONE" if complete else "PARTIAL"
    if chit:
        return "C1-%s/R-CONTROL-HIT" % state
    if harn:
        return "C1-%s/R-HARNESS" % state
    if crash or hother:
        return "C1-%s/R-CRASH" % state
    if not complete:
        return "C1-PARTIAL/R-INCONC"
    if g1 != "PASS":
        return "C1-DONE/R-INCONC-NO-G1"
    if k == 0:
        return "C1-DONE/R-NULL"
    if not okh:
        return "C1-DONE/E-UNMET/DIRECT"
    if eng == "no":
        return "C1-DONE/E-UNMET/ENGINE"
    if eng == "none":
        return "C1-DONE/R-HIT-CANDIDATE"
    return "C1-DONE/R-HIT" if k >= 8 else "C1-DONE/R-HIT-FEW"


# The exact stderr of the two Hermit failures of window 1 (window 1, out/C1.u.i0172.stderr and out/C1.u.i0284.stderr), used as selftest fixtures.
_REAL_HEAD = """hermit: virtual-time epoch=2026-01-01T00:00:00+00:00 source=explicit; reproduce with --epoch=2026-01-01T00:00:00+00:00
{W1}
{W2}
HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"go","sdk_version":"0.1.0","proc":"p1","catalog_hash":"4aac0c0e2aba0b11","signal_handlers":false}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.cancel_overlapped_scan","type":"sometimes","must_hit":true,"msg":"the cancel flag flipped while Scan was running"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.conn_handed_to_waiter","type":"sometimes","must_hit":true,"msg":"a query ran Next while an earlier query's Scan was in flight"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.harness.all_queries_ran","type":"always","must_hit":true,"msg":"not every query was run and counted"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.harness.conn_exclusive","type":"always","must_hit":true,"msg":"two queries owned the one connection at once"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.harness.error_without_cancel","type":"always","must_hit":true,"msg":"an error arrived although the query was not cancelled"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.scan_error_after_cancel","type":"reachable","must_hit":true,"msg":"Scan returned an error, which a cancelled query may do"}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"go.sql.scan_returns_own_row","type":"always","must_hit":true,"msg":"Scan returned a value that is not the row of its own query"}
"""
_REAL_SHAPE_B = _REAL_HEAD.replace(
    "{W1}", "2026-10-18T00:21:43.136309Z  WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL signum=5 effect=handler-installed-but-never-invoked reason=ptrace consumes every SIGTRAP (syscall/seccomp stops, breakpoints)").replace(
    "{W2}", "2026-10-18T00:21:43.139999Z  WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL signum=16 effect=handler-installed-but-never-invoked reason=reverie uses it as PERF_EVENT_SIGNAL, the PMU preemption timer") + """HERMIT_SKID_OVERSHOOT rcb_actual=1438054 rcb_target=1438053 skid_margin=25 overshoot=1
2026-10-18T00:21:47.246463Z  WARN reverie_ptrace::timer: Precise timer interrupt arrived after target: actual 1438054 > target 1438053; delivering timer event at the observed counter

thread 'main' (1) panicked at detcore/src/lib.rs:348:9:
HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot! Clock_value: 1438054. Stepped forward 1252 RCBs, but should have trapped at 1251
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
HERMIT_CLEANUP_UNCONFIRMED: backend phase Some(Poisoned); namespace retirement observed=true; no backend/recorder cleanup success is claimed
HERMIT_INTERNAL_FAILURE class=container-child-panic
Error: Hermit cleanup remains unconfirmed (Poisoned, recovery 1): ordinary operation panicked while polling; owner is poisoned
"""
_REAL_SHAPE_A = _REAL_HEAD.replace(
    "{W1}", "2026-10-18T00:46:26.607338Z  WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL signum=5 effect=handler-installed-but-never-invoked reason=ptrace consumes every SIGTRAP (syscall/seccomp stops, breakpoints)").replace(
    "{W2}", "2026-10-18T00:46:26.610878Z  WARN detcore::syscalls::signal: HERMIT_APPROPRIATED_SIGNAL signum=16 effect=handler-installed-but-never-invoked reason=reverie uses it as PERF_EVENT_SIGNAL, the PMU preemption timer") + """HERMIT_SKID_OVERSHOOT rcb_actual=2198844 rcb_target=2198843 skid_margin=25 overshoot=1
2026-10-18T00:46:30.500892Z  WARN reverie_ptrace::timer: Precise timer interrupt arrived after target: actual 2198844 > target 2198843; delivering timer event at the observed counter

thread 'main' (1) panicked at detcore/src/lib.rs:348:9:
HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot! Clock_value: 2198844. Stepped forward 435 RCBs, but should have trapped at 434
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
HERMIT_TASK_PANIC tid=7 exit=101 message=HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot! Clock_value: 2198844. Stepped forward 435 RCBs, but should have trapped at 434
HERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)
Error: Sandbox container exited unexpectedly: Exited(101)
"""


def selftest():
    # statistics anchored to the design (8.1): k = 8 is the least A-u hit count at N = 2,000 with both controls at 0
    assert abs(100 * sclib.wilson(0, 2000)[1] - 0.1917) < 5e-5
    assert [k for k in range(0, 30) if r_hit(k, 2000, 2000, 2000)][0] == 8 and not r_hit(7, 2000, 2000, 2000)
    assert not r_hit(0, 2000, 2000, 2000) and not r_hit(8, 0, 2000, 2000)
    # a few Hermit failures per arm (outside the rated runs) move the least hit count by at most one: k = 8 still passes down to N = 1,950 per arm
    assert all(r_hit(8, n, n, n) for n in range(1950, 2001)) and not any(r_hit(7, n, n, n) for n in range(1950, 2001))
    if os.environ.get("C_TEST") != "1":
        assert CFG == {"n": 2000} and C2_MAX == 100 and TIMEOUT_HALT == 20 and HERMIT_FAIL_HALT == 40 and ALLBAD_RUNS == 15      # the frozen sizes (the Mac harness may override n with C_TEST=1)
    saved = dict(CFG)
    CFG.update(n=2000)
    old = dict(CFG)
    CFG.update(n=6)
    try:
        # rotation, as in N
        assert rotation(1) == ["u", "c", "cp"] and rotation(2) == ["c", "cp", "u"] and rotation(3) == ["cp", "u", "c"] and rotation(4) == rotation(1)
        # next: empty, partial block, complete; one identical seed list for the three arms
        assert next_cells([]) == ["RUN C1 u 330001 1", "RUN C1 c 330001 1", "RUN C1 cp 330001 1"]
        assert next_cells([_crow("u", 1), _crow("c", 1)]) == ["RUN C1 cp 330001 1"]
        assert next_cells(_all_cells(1)) == ["RUN C1 c 330002 2", "RUN C1 cp 330002 2", "RUN C1 u 330002 2"]
        assert next_cells(_all_cells(6)) == ["DONE"]
        # a failed cell of any class is a decided cell: it is never run again and never replaced
        texts = {}

        def hurt(r, **kw):
            r.update(_skid_kw())
            r.update(kw)
            texts[r["id"]] = g_eval._SKID_ERR
            return r
        rse = lambda i: texts.get(i, "")        # noqa: E731
        failed = [hurt(_crow("u", 2), pf=7), _crow("c", 2, rc=124), _crow("cp", 2, rc=2, fatal=1)]
        assert next_cells(_all_cells(2, skip={("u", 2), ("c", 2), ("cp", 2)}) + failed) == ["RUN C1 cp 330003 3", "RUN C1 u 330003 3", "RUN C1 c 330003 3"]
        # integrity
        assert next_cells(_all_cells(1) + [_crow("u", 1)])[0].startswith("ERROR duplicate")
        assert next_cells([_row("C1", "u", 330099)])[0].startswith("ERROR")
        assert next_cells([_row("C1S", "u", 332001)])[0].startswith("ERROR unknown step")
        assert next_cells([], bad_lines=[4])[0] == "ERROR malformed runs.tsv line 4"
        assert malformed("a\tb\n") == [1] and malformed("") == [] and malformed("\t".join(["x"] * len(sclib.COLS)) + "\n") == []
        assert malformed("\t".join(["x"] * (len(sclib.COLS) + 1)) + "\n") == [1] and malformed("ok\n" + "\t".join(["x"] * len(sclib.COLS)) + "\n") == [1]      # a swallowed neighbour has too many columns
        assert integrity([_row("C1", "u", 330001, build="v")]) and integrity([_row("C1", "u", 330001, mode="yield")]) and integrity([_row("C1", "u", 330001, 2)])
        assert integrity([_row("C2D", "c", 330001, 1)]) and integrity([_row("C2D", "u", 330001, 4)]) and not integrity([_row("C2D", "u", 330001, 3)])
        assert integrity([_row("G0", "u", 1)])
        # infrastructure halts only: no result (a hit, a control hit, a harness failure, a crash, a Hermit failure) stops a window by itself
        assert halts(_all_cells(2)) is None
        assert halts([_crow("c", 1, pf=1), _crow("u", 2, pf=5), _crow("u", 3, rc=3, hf=1), _crow("u", 4, rc=2, fatal=1), hurt(_crow("u", 5))]) is None
        bad = [_crow("u", i, rc=2, fatal=1) for i in range(1, ALLBAD_RUNS + 1)]
        assert halts(bad[:ALLBAD_RUNS - 1]) is None and halts(bad) == "C1-HALT/ALL-BAD"
        assert halts(bad[:ALLBAD_RUNS - 1] + [_crow("u", 99)]) is None and halts([_crow("u", 1)] + bad[1:]) is None and halts(bad + _all_cells(1)) == "C1-HALT/ALL-BAD"
        assert halts([_crow("c", 1)] + [_crow("u", i, rc=124) for i in range(2, TIMEOUT_HALT + 1)]) is None
        assert halts([_crow("c", 1)] + [_crow("u", i, rc=124) for i in range(2, TIMEOUT_HALT + 2)]) == "C1-HALT/TIMEOUT-RATE"
        assert halts([_crow("u", 1)] + [hurt(_crow("c", i)) for i in range(2, HERMIT_FAIL_HALT + 1)]) is None
        assert halts([_crow("u", 1)] + [hurt(_crow("c", i)) for i in range(1, HERMIT_FAIL_HALT + 1)]) == "C1-HALT/HERMIT-FAIL-RATE"
        assert halts([_row("C2D", "u", 330001 + i, 1, rc=124) for i in range(TIMEOUT_HALT)]) == "C1-HALT/TIMEOUT-RATE"
        assert halts([_crow("u", 1), _crow("u", 2)] + [_crow("u", i, rc=124) for i in range(3, 3 + TIMEOUT_HALT)]) == "C1-HALT/TIMEOUT-RATE"
        # a hit that is not classed HIT (a crashed run that carries it) still counts as a control hit in the analysis and the outcome
        crashed = _all_cells(6)
        crashed[1].update(rc=2, fatal=1, pf=3)
        an_c = analyse(crashed, rse)
        assert crashed[1]["arm"] == "c" and an_c["arms"]["c"]["hit_detail_all"] == 1 and an_c["arms"]["c"]["hit"] == 0 and an_c["arms"]["c"]["crash"] == 1
        assert outcome(an_c, None, False, "PASS") == "C1-DONE/R-CONTROL-HIT"
        an_u = analyse(_all_cells(6), rse)
        assert an_u["complete"] and an_u["arms"]["u"]["rated"] == 6 and outcome(an_u, None, False, "PASS") == "C1-DONE/R-NULL"
        # Hermit failures: counted per arm outside the rated runs, never replaced; the SKID_OVERSHOOT signature is a ledger suffix, any other shape blocks (R-CRASH)
        sk = _all_cells(6, skip={("u", 2)}) + [hurt(_crow("u", 2), pf=7)]
        an_s = analyse(sk, rse)
        assert an_s["complete"] and an_s["arms"]["u"]["rated"] == 5 and an_s["arms"]["u"]["hermit"] == 1 and an_s["arms"]["u"]["hermit_skid"] == 1 and an_s["arms"]["u"]["hit"] == 0
        assert outcome(an_s, None, False, "PASS") == "C1-DONE/R-NULL+R-CRASH-hermit(u=1,c=0,cp=0)"
        other = _crow("c", 2, **_skid_kw())
        texts[other["id"]] = g_eval._SKID_ERR + "HERMIT_CLEANUP_UNCONFIRMED\n"
        an_o = analyse(_all_cells(6, skip={("c", 2)}) + [other], rse)
        assert an_o["arms"]["c"]["hermit_other"] == 1 and outcome(an_o, None, False, "PASS") == "C1-DONE/R-CRASH"
        # rev 1b: shape B, fixtures built from the real stderr of C1.u.i0172 (shape B) and C1.u.i0284 (shape A). One selection rule (the strict signature), never a run-by-run decision.
        row_b = _crow("u", 2, **_skid_kw())
        row_a = _crow("c", 2, **_skid_kw())
        assert g_eval.skid_run(row_a, _REAL_SHAPE_A) and not g_eval.skid_run(row_b, _REAL_SHAPE_B)      # rev 1a: shape A is the skid ledger, shape B blocks
        assert skid_b_run(row_b, _REAL_SHAPE_B) and not skid_b_run(row_a, _REAL_SHAPE_A)                # shape B is exactly shape B; shape A is not shape B
        assert skid_b_run(row_b, _REAL_SHAPE_B.replace("thread 'main' (1)", "thread 'tokio-runtime-worker' (7)"))        # the panic may land on ANY thread
        assert skid_b_run(dict(row_b, arm="cp"), _REAL_SHAPE_B) and skid_b_run(dict(row_b, arm="c"), _REAL_SHAPE_B)       # the same rule for every arm

        def without(text, prefix):
            return "".join(ln + "\n" for ln in text.splitlines() if not ln.startswith(prefix))
        near = {
            "another panic line": _REAL_SHAPE_B.replace("lib.rs:348:9", "lib.rs:349:9"),
            "another file": _REAL_SHAPE_B.replace("detcore/src/lib.rs:348", "detcore/src/other.rs:348"),
            "no cleanup line": without(_REAL_SHAPE_B, "HERMIT_CLEANUP_UNCONFIRMED"),
            "cleanup phase not Poisoned": _REAL_SHAPE_B.replace("Some(Poisoned)", "Some(Recovering)"),
            "namespace retirement not observed": _REAL_SHAPE_B.replace("observed=true", "observed=false"),
            "an extra task panic marker": _REAL_SHAPE_B + "HERMIT_TASK_PANIC tid=7 exit=101 message=x\n",
            "an extra Hermit marker": _REAL_SHAPE_B + "HERMIT_FOO bar\n",
            "an extra internal failure": _REAL_SHAPE_B + "HERMIT_INTERNAL_FAILURE class=container-child-exit status=Exited(101)\n",
            "another internal class": _REAL_SHAPE_B.replace("container-child-panic", "container-child-exit"),
            "no internal failure line": without(_REAL_SHAPE_B, "HERMIT_INTERNAL_FAILURE"),
            "no rcb line": without(_REAL_SHAPE_B, "HERMIT_SKID_OVERSHOOT rcb_actual"),
            "no prehook message": _REAL_SHAPE_B.replace("HERMIT_SKID_OVERSHOOT prehook: PMU RCB overshoot!", "something else"),
            "another error line": _REAL_SHAPE_B.replace("Error: Hermit cleanup remains unconfirmed (Poisoned", "Error: Hermit cleanup remains unconfirmed (Recovering"),
            "no error line": without(_REAL_SHAPE_B, "Error:"),
            "an extra error line": _REAL_SHAPE_B + "Error: something else\n",
            "a second panic": _REAL_SHAPE_B + "thread 'main' (1) panicked at detcore/src/lib.rs:348:9:\n",
            "a deadlock report": _REAL_SHAPE_B + "Deadlock detected\n",
            "a usage text": _REAL_SHAPE_B + "Usage: hermit run\n",
            "nothing": "",
            "shape A": _REAL_SHAPE_A,
        }
        for name, text in near.items():
            assert not skid_b_run(row_b, text), name
        for kw in (dict(rc=101), dict(rc=124), dict(rc=1), dict(rc=0), dict(rc=2), dict(fatal=1), dict(dead=1)):
            assert not skid_b_run(dict(row_b, **kw), _REAL_SHAPE_B), kw                                  # exit status not 125, a Go fatal or a deadlock fact
        assert leftover_clean("CLEAN\n") and leftover_clean("CLEAN") and leftover_clean("CLEAN\nnotes")
        assert not leftover_clean("") and not leftover_clean("DIRTY a hermit process\n") and not leftover_clean("clean\n") and not leftover_clean("CLEAN extra\n") and not leftover_clean("\nCLEAN\n")
        sb = _all_cells(6, skip={("u", 2)}) + [row_b]
        tb = lambda i: _REAL_SHAPE_B if i == row_b["id"] else ""            # noqa: E731
        assert outcome(analyse(sb, tb), None, False, "PASS") == "C1-DONE/R-CRASH"                  # the mechanical code (rev 1a): shape B blocks
        clean_b = lambda r: leftover_clean("CLEAN\n") and skid_b_run(r, tb(r["id"]))            # noqa: E731
        dirty_b = lambda r: leftover_clean("DIRTY: a hermit process\n") and skid_b_run(r, tb(r["id"]))      # noqa: E731
        assert outcome(analyse(sb, tb, None, clean_b), None, False, "PASS") == "C1-DONE/R-NULL+R-CRASH-hermit(u=1,c=0,cp=0)"
        assert outcome(analyse(sb, tb, None, dirty_b), None, False, "PASS") == "C1-DONE/R-CRASH"           # a dirty leftover scan keeps shape B blocking
        assert analyse(sb, tb, None, clean_b)["arms"]["u"]["hermit_skid"] == 1 and analyse(sb, tb, None, clean_b)["arms"]["u"]["hermit_other"] == 0
        sbc = _all_cells(6, skip={("c", 2)}) + [dict(row_b, arm="c", id="C1.c.%d.1" % (SEED0 + 2), seed=str(SEED0 + 2))]
        tbc = lambda i: _REAL_SHAPE_B                                              # noqa: E731
        assert outcome(analyse(sbc, tbc, None, lambda r: skid_b_run(r, tbc(r["id"]))), None, False, "PASS") == "C1-DONE/R-NULL+R-CRASH-hermit(u=0,c=1,cp=0)"
        oid = "C1.c.%d.1" % (SEED0 + 3)
        tmix = lambda i: (g_eval._SKID_ERR + "HERMIT_CLEANUP_UNCONFIRMED\n") if i == oid else _REAL_SHAPE_B       # noqa: E731
        am = analyse(_all_cells(6, skip={("u", 2), ("c", 3)}) + [row_b, dict(other, seed=str(SEED0 + 3), id=oid)], tmix, None, lambda r: skid_b_run(r, tmix(r["id"])))
        assert am["arms"]["u"]["hermit_skid"] == 1 and am["arms"]["c"]["hermit_other"] == 1 and outcome(am, None, False, "PASS") == "C1-DONE/R-CRASH+R-CRASH-hermit(u=1,c=0,cp=0)"       # another shape stays blocking
        # probe and pause arithmetic
        assert probe(12, 2, 24) == ("OK", 12.0) and probe(12, 0, 0) == ("OK", None)
        assert probe(5, 2, 24) == ("PAUSE-RATE", 12.0) and probe(6, 2, 24) == ("OK", 12.0)       # 12 s a block against 2.0 x 6 s: not over; against 2.0 x 5 s: over
        assert probe(12, 5, 5 * 24 + 1)[0] == "PAUSE-RATE" and probe(12, 5, 5 * 24)[0] == "OK"      # the factor is 2.0 and the edge is not a pause
        assert pause_hours([], [], 12, 6 * 12 + 2 * OVERHEAD_S, 3)[0] is False and pause_hours([], [], 12, 6 * 12 + 2 * OVERHEAD_S - 1, 3)[0] is True
        assert pause_hours(_all_cells(4), [100], 12, 100 + 2 * 12 + OVERHEAD_S, 3) == (False, 100, 2 * 12 + OVERHEAD_S)
        assert pause_hours(_all_cells(4), [100], 12, 100 + 2 * 12 + OVERHEAD_S - 1, 3)[0] is True
        assert pause_hours(_all_cells(6), [10 ** 6], 12, 1, 3) == (True, 10 ** 6, 0)
        assert blocks_decided(_all_cells(3, skip={("u", 2)})) == 2
        # hits for C2
        hr = [_crow("u", 1, pf=2), _crow("u", 3, pf=1), _crow("c", 2), _crow("u", 5, rc=2, fatal=1, pf=1)]
        assert hit_seeds(hr) == (2, [330001, 330003])
    finally:
        CFG.update(old)
    # the cap on C2 seeds and the full-size defaults
    assert CFG["n"] == 2000 and C2_MAX == 100
    many = [_row("C1", "u", SEED0 + i, pf=1) for i in range(1, 151)]
    assert hit_seeds(many) == (150, [SEED0 + i for i in range(1, 101)])
    # the plan (mechanical): equal 4.0 s means -> BLOCK_S 13, 5 balanced windows of 400; fast runs -> fewer windows; the effective-rows reader drops nothing here
    g0 = []
    for arm in ARMS:
        for i, s in enumerate(range(339521, 339531)):
            for rep in (1, 2, 3):
                g0.append(g_eval._row("G0", arm, s, rep, ms=4000, ovlt=1, errh=1))
    p = plan(g0, 339521)
    assert p["nwin"] == 5 and p["bpw"] == 400 and p["ranges"][0] == (1, 400) and p["ranges"][-1] == (1601, 2000), p
    assert p["block_s"] == 13 and abs(p["run_hours"] - 2000 * 12.75 / 3600.0) < 1e-3 and abs(p["total_hours"] - (2000 * 12.75 / 3600.0 + 5 * 420 / 3600.0)) < 1e-3
    p4 = plan(g0, 339521, hard_s=7800)
    assert p4["nwin"] == 4 and p4["bpw"] == 500 and p4["ranges"][-1] == (1501, 2000), p4
    fast = [dict(r, ms=1000) for r in g0]
    pf_ = plan(fast, 339521)
    assert pf_["block_s"] == 4 and pf_["nwin"] == 2 and pf_["bpw"] == 1000 and pf_["ranges"] == [(1, 1000), (1001, 2000)], pf_
    assert plan([r for r in g0 if r["arm"] != "cp"], 339521) is None
    slow = plan([dict(r, ms=60000) for r in g0], 339521)
    assert slow["bpw"] == 27 and slow["nwin"] == 75 and slow["block_s"] == 181 and sum(b - a + 1 for a, b in slow["ranges"]) == 2000      # 95% sizing: 5013 // 181 = 27 (at 100% it would be 29)
    # the previous-windows bundle
    CFG.update(n=2)
    try:
        rt = "".join("\t".join(str(r[c]) for c in sclib.COLS) + "\n" for r in _all_cells(2))
        wt = "1\t5000\t250\n2\t100\t250\n"
        sh = "%s runs.tsv\n%s walls.tsv\n" % (sha_of(rt), sha_of(wt))
        assert prev_ok(rt, wt, sh) == "OK rows=6 blocks=2", prev_ok(rt, wt, sh)
        assert prev_ok(rt + "x", wt, sh).startswith("BAD sha256") and prev_ok(rt, wt + " ", sh).startswith("BAD sha256") and prev_ok(rt, wt, "").startswith("BAD sha256")
        bad_rt = rt + "a\tb\n"
        assert prev_ok(bad_rt, wt, "%s runs.tsv\n%s walls.tsv\n" % (sha_of(bad_rt), sha_of(wt))).startswith("BAD malformed")
        dup = rt + rt.splitlines()[0] + "\n"
        assert prev_ok(dup, wt, "%s runs.tsv\n%s walls.tsv\n" % (sha_of(dup), sha_of(wt))).startswith("BAD duplicate")
        assert prev_ok(rt, wt, sh, 250) == "OK rows=6 blocks=2" and prev_ok(rt, wt, sh, 500).startswith("BAD qw mismatch")
        wt2 = "1\t5000\t250\n2\t100\t500\n"
        assert prev_ok(rt, wt2, "%s runs.tsv\n%s walls.tsv\n" % (sha_of(rt), sha_of(wt2)), 250).startswith("BAD qw mismatch")
        assert prev_ok(rt, wt2, "%s runs.tsv\n%s walls.tsv\n" % (sha_of(rt), sha_of(wt2))) == "OK rows=6 blocks=2"
    finally:
        CFG.update(old)
    assert walls_of(wt) == [5000, 100] and walls_of("x\ty\n") == [] and qws_of(wt) == ["250", "250"] and qws_of("1\t2\n") == []
    # witness class, the direct reruns and the engine records
    raw = ('HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"go.sql.scan_returns_own_row","hits":                3026,"trues":                3025,"falses":                   1,'
           '"first_false":{"thread":"go","ord":                 215,"vals":[2069,1063,1063,1]}                      ,"first_true":{"thread":"go","ord":                   1,"vals":[1001,1001,1001,0]}}\n')
    assert witness_class(raw) == 1 and witness_class(raw.replace("1063,1]}", "1064,2]}")) == 2 and witness_class("") is None and witness_class(raw.replace("first_false", "x")) is None
    assert witness_class(raw.replace("scan_returns_own_row", "other")) is None
    h = _crow("u", 1, pf=1, stdout_sha="sA", fp="fA")
    good = [_row("C2D", "u", 330001, k, pf=1, stdout_sha="sA", fp="fA") for k in (1, 2, 3)]
    assert confirmation(h, good) == (3, 3) and confirmation(h, good[:2]) == (2, 2) and confirmation(h, []) == (0, 0)
    assert confirmation(h, good[:2] + [dict(good[2], fp="fB")]) == (2, 3) and confirmation(h, good[:2] + [dict(good[2], pf=0)]) == (2, 3)
    assert confirmation(h, good[:2] + [dict(good[2], stdout_sha="sB")]) == (2, 3) and confirmation(h, good[:2] + [dict(good[2], seed="5")]) == (2, 2)
    t_ok = json.dumps([{"outcome": {"status": "failed", "trials": 1, "failure": {"message": "property violated at sched-seed 330003 (always 'x' was false 4 time(s)); reproduced 3/3"}}}])
    r_ok = "replaying 6 steps on target `scau` (backend hermit)\n    3. " + REPLAY_LINE + "\nreproduced: the property is still violated\n"
    assert engine_result(t_ok, r_ok, 330003) == (True, True) and engine_result("", "", 330003) == (None, None) and engine_result("  \n", r_ok, 330003) == (None, None)
    assert engine_result(t_ok, r_ok, 330004) == (False, True)                                         # the engine failed at another seed: not a reproduction of this one
    assert engine_result(t_ok.replace("reproduced 3/3", "reproduced 2/3"), r_ok, 330003) == (False, True)
    assert engine_result(t_ok.replace('"failed"', '"passed"'), r_ok, 330003) == (False, True)
    assert engine_result(t_ok, r_ok.replace("schedule matches the recording: yes", "schedule matches the recording: no"), 330003) == (True, False)
    assert engine_result(t_ok, "", 330003) == (True, False) and engine_result("not json", r_ok, 330003) == (False, True) and engine_result("[]", r_ok, 330003) == (False, True)
    assert engine_result(json.dumps([{"outcome": {"status": "failed", "failure": None}}]), r_ok, 330003) == (False, True)
    # steps.tsv facts and the window end
    m = window_meta("10\tARGS mode=count qw=250 win=1\n11\tGATES-PASSED\n20\tHALT-RULE C1-HALT/ALL-BAD x\n30\tWINDOW-WALL 20\n")
    assert m["args"] == "mode=count qw=250 win=1" and window_end(m) == "C1-HALT/ALL-BAD" and m["wall"] == 20
    assert window_end(window_meta("1\tGATE-FAIL SUDO\n")) == "GATE-FAIL/SUDO" and window_end(window_meta("1\tWINDOW-FULL blocks=4\n")) == "FULL"
    assert window_end(window_meta("1\tC2E-COMPLETE x\n")) == "C2E-COMPLETE" and window_end(window_meta("1\tC2D-COMPLETE x\n")) == "C2D-COMPLETE" and window_end(window_meta("1\tC1-COMPLETE x\n")) == "COMPLETE"
    assert window_end(window_meta("1\tPAUSE-BUDGET C1-PAUSE/HOURS x\n")) == "C1-PAUSE/HOURS"
    assert window_end(window_meta("1\tBUDGET: past the no-new-work line\n")) == "BUDGET" and window_end(window_meta("")) == "NO-END-LINE"
    assert window_meta("5\tx\n9\ty\n")["wall"] == 4 and window_meta("1\tBIN-CHANGED u-shim\n")["bin_changed"]
    # the outcome table: exhaustive over the inputs that matter, against a flat oracle
    n = 0
    base_arm = dict(rated=2000, hit=0, timeout=0, crash=0, harness=0, hermit=0, hermit_skid=0, hermit_other=0, hit_detail_all=0, decided=2000, missing=0, complete=True)
    for gate in (None, "SUDO"):
        for binchg in (False, True):
            for integ in (False, True):
                for complete in (True, False):
                    for chit in (0, 1):
                        for harn in (0, 1):
                            for crash in (0, 1):
                                for hother in (0, 1):
                                    for k in (0, 1, 9):
                                        for g1 in ("PASS", None):
                                            for okh in (True, False):
                                                for eng in ("yes", "none", "no"):
                                                    arms = {a: dict(base_arm) for a in ARMS}
                                                    arms["u"]["hit"] = k
                                                    arms["c"]["hit_detail_all"] = chit
                                                    arms["u"]["harness"] = harn
                                                    arms["cp"]["crash"] = crash
                                                    arms["c"]["hermit_other"] = hother
                                                    ev = {"yes": True, "none": None, "no": False}[eng]
                                                    an = {"integrity": ["x"] if integ else [], "arms": arms, "complete": complete,
                                                          "hits": [{"ok": okh, "eng": ev, "rep": ev}] * (1 if k else 0)}
                                                    got = outcome(an, gate, binchg, g1, any_cells=True)
                                                    want = _oracle(gate, binchg, integ, complete, chit, harn, crash, hother, k, g1, okh, eng, 2000)
                                                    assert got == want, (gate, binchg, integ, complete, chit, harn, crash, hother, k, g1, okh, eng, got, want)
                                                    assert "NATIVE-EASY" not in got
                                                    n += 1
    an0 = {"integrity": [], "arms": {a: dict(base_arm) for a in ARMS}, "complete": False, "hits": []}
    assert outcome(an0, "SUDO", False, None, any_cells=False) == "C1-NOT-STARTED/SUDO"
    an1 = {"integrity": [], "arms": {a: dict(base_arm) for a in ARMS}, "complete": True, "hits": []}
    an1["arms"]["u"].update(hermit_skid=2, hermit=2, timeout=1)
    assert outcome(an1, None, False, "PASS") == "C1-DONE/R-NULL+R-CRASH-hermit(u=2,c=0,cp=0)+TIMEOUTS(u=1,c=0,cp=0)"
    an2 = {"integrity": [], "arms": {a: dict(base_arm) for a in ARMS}, "complete": True, "hits": [{"ok": True, "eng": True, "rep": True}] * 8}
    an2["arms"]["u"]["hit"] = 8
    assert outcome(an2, None, False, "PASS") == "C1-DONE/R-HIT"
    an2["arms"]["u"]["hit"] = 7
    assert outcome(an2, None, False, "PASS") == "C1-DONE/R-HIT-FEW"
    an2["arms"]["u"]["hit"] = 8
    an2["arms"]["c"]["rated"] = 100            # a control with few rated runs has a wide upper bound: the rate comparison is not made
    assert outcome(an2, None, False, "PASS") == "C1-DONE/R-HIT-FEW"
    an3 = {"integrity": [], "arms": {a: dict(base_arm) for a in ARMS}, "complete": True, "hits": [{"ok": False, "reruns": 0, "eng": None, "rep": None}]}
    an3["arms"]["u"]["hit"] = 1
    assert outcome(an3, None, False, "PASS") == "C1-DONE/C2-PENDING"
    an3["hits"] = [{"ok": False, "reruns": 3, "eng": None, "rep": None}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/E-UNMET/DIRECT"
    an3["hits"] = [{"ok": False, "reruns": 3, "eng": False, "rep": False}]            # the direct failure comes first
    assert outcome(an3, None, False, "PASS") == "C1-DONE/E-UNMET/DIRECT"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": None, "rep": None}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/R-HIT-CANDIDATE"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": True, "rep": None}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/R-HIT-CANDIDATE"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": True, "rep": True}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/R-HIT-FEW"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": True, "rep": False}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/E-UNMET/ENGINE"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": False, "rep": True}]       # the engine did not fail at the hit's own seed: unmet even if a replay matched
    assert outcome(an3, None, False, "PASS") == "C1-DONE/E-UNMET/ENGINE"
    an3["hits"] = [{"ok": True, "reruns": 3, "eng": False, "rep": None}]
    assert outcome(an3, None, False, "PASS") == "C1-DONE/E-UNMET/ENGINE"
    # R-HIT-NATIVE-EASY: the branch returns the native-easy code, never R-HIT-FEW, if the flag were ever true (it is false: GON's N-u is not 'high')
    global NATIVE_HIGH
    an2["arms"]["c"]["rated"] = 2000
    an2["arms"]["u"]["hit"] = 8
    NATIVE_HIGH = True
    try:
        assert outcome(an2, None, False, "PASS") == "C1-DONE/R-HIT-NATIVE-EASY"
    finally:
        NATIVE_HIGH = False
    assert outcome(an2, None, False, "PASS") == "C1-DONE/R-HIT"
    # k = 150 A-u hits: C2 confirms the first m = min(k, 100) in seed order; the E-conditions are judged on those m; k and the rate come from all the counted runs
    CFG.update(n=150)
    raw_hit = ('HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"go.sql.scan_returns_own_row","hits": 9,"trues": 8,"falses": 1,'
               '"first_false":{"thread":"go","ord": 2,"vals":[7,1,1,1]}}\n')
    big = [_crow("u", i, pf=1) for i in range(1, 151)] + [_crow(a, i) for a in CONTROLS for i in range(1, 151)]
    c2d150 = [_row("C2D", "u", SEED0 + i, k, pf=1) for i in range(1, 101) for k in (1, 2, 3)]
    eng150 = {SEED0 + i: (True, True) for i in range(1, 101)}
    se150 = lambda i: raw_hit                                                       # noqa: E731
    a150 = analyse(big + c2d150, se150, eng150)
    assert a150["k_total"] == 150 and a150["m"] == 100 and len(a150["hits"]) == 100 and a150["arms"]["u"]["hit"] == 150 and a150["complete"]
    assert [h["seed"] for h in a150["hits"]] == [str(SEED0 + i) for i in range(1, 101)]            # the first 100 in seed order, nothing chosen
    assert outcome(a150, None, False, "PASS") == "C1-DONE/R-HIT"
    assert analyse(big, se150, {})["hits"][0]["reruns"] == 0 and outcome(analyse(big, se150, {}), None, False, "PASS") == "C1-DONE/C2-PENDING"      # C2 not yet run
    assert outcome(analyse(big + c2d150, se150, {}), None, False, "PASS") == "C1-DONE/R-HIT-CANDIDATE"
    bad_direct = [dict(r, fp="other") if r["seed"] == str(SEED0 + 57) and r["rep"] == "2" else r for r in c2d150]
    assert outcome(analyse(big + bad_direct, se150, eng150), None, False, "PASS") == "C1-DONE/E-UNMET/DIRECT"
    eng_bad = dict(eng150)
    eng_bad[SEED0 + 100] = (False, True)
    assert outcome(analyse(big + c2d150, se150, eng_bad), None, False, "PASS") == "C1-DONE/E-UNMET/ENGINE"
    eng_bad[SEED0 + 100] = (True, False)
    assert outcome(analyse(big + c2d150, se150, eng_bad), None, False, "PASS") == "C1-DONE/E-UNMET/ENGINE"
    out101 = [r for r in c2d150 if r["seed"] != str(SEED0 + 101)] + [_row("C2D", "u", SEED0 + 101, 1, pf=0)]       # a rerun of the 101st hit is outside the sample: ignored
    assert outcome(analyse(big + out101, se150, eng150), None, False, "PASS") == "C1-DONE/R-HIT"
    assert outcome(analyse(big + c2d150, lambda i: raw_hit.replace("[7,1,1,1]", "[7,1,1,3]") if i == big[4]["id"] else raw_hit, eng150), None, False, "PASS") == "C1-DONE/E-UNMET/DIRECT"
    CFG.update(n=2000)
    CFG.update(saved)
    print("selftest ok: statistics anchored, plan, halts, hits and Hermit failures, %d outcome combinations" % n)


# ---------------------------------------------------------------- CLI
def main(argv):
    mode = argv[1] if len(argv) > 1 else ""
    try:
        if mode == "next":
            text_prev, text_cur = load_text_rows(argv[2])
            rows = sclib.parse_rows(text_prev) + sclib.parse_rows(text_cur)
            print("\n".join(next_cells(rows, malformed(text_prev) + malformed(text_cur))))
        elif mode == "halts":
            h = halts(sclib.parse_rows(g_eval.read_text(argv[2])))
            print("OK" if h is None else "HALT " + h)
        elif mode == "probe":
            v, b = probe(float(argv[2]), int(argv[3]), int(argv[4]))
            print("%s b=%s plan=%s" % (v, "NA" if b is None else "%.1f" % b, argv[2]))
        elif mode == "pausehours":
            d = argv[2]
            rows = sclib.parse_rows(g_eval.read_text(os.path.join(d, "runs.tsv")))
            walls = walls_of(g_eval.read_text(os.path.join(d, "walls.tsv")))
            pz, used, need = pause_hours(rows, walls, float(argv[3]), float(argv[4]), int(argv[5]))
            print("%s used=%d need=%d cap=%s" % ("PAUSE-HOURS" if pz else "OK", used, need, argv[4]))
        elif mode == "prevcheck":
            d = argv[2]
            print(prev_ok(g_eval.read_text(os.path.join(d, "runs.tsv")), g_eval.read_text(os.path.join(d, "walls.tsv")), g_eval.read_text(os.path.join(d, "prev.sha")),
                          argv[3] if len(argv) > 3 else None))
        elif mode == "hits":
            rows = sclib.parse_rows(g_eval.read_text(os.path.join(argv[2], "runs.tsv")))
            total, seeds = hit_seeds(rows)
            print("NONE" if total == 0 else "HITS %d %d" % (total, len(seeds)))
            for s in seeds:
                print(s)
        elif mode == "plan":
            rows = sclib.parse_rows(g_eval.read_text(argv[2]))
            p = plan(rows, int(argv[3]), g_eval.stderr_reader(g_eval.out_dir_of(argv[2])), int(argv[4]) if len(argv) > 4 else WIN_HARD_S)
            if p is None:
                print("PLAN-ERROR an arm has no rated G0 run")
                return 1
            print("mean G0 wall per run (ms): " + ", ".join("%s=%.0f" % (a, p["mean_ms"][a]) for a in ARMS))
            print("BLOCK_S=%d (sum of the three arm means plus %.2f s of driver time per run, rounded up) | available seconds per window %.0f | MAXBLOCKS=%d | windows=%d" % (
                p["block_s"], DRIVER_S_PER_RUN, p["avail_s"], p["bpw"], p["nwin"]))
            print("run hours %.2f | with %d s per window overhead %.2f" % (p["run_hours"], OVERHEAD_S, p["total_hours"]))
            for i, (a, b) in enumerate(p["ranges"], 1):
                print("window %d: indices %d-%d (seeds %d-%d, %d runs)" % (i, a, b, SEED0 + a, SEED0 + b, 3 * (b - a + 1)))
        elif mode == "prepare":
            cmd_prepare(argv[2], argv[3:])
        elif mode == "verdict":
            main_verdict(argv[2:])
        elif mode == "selftest":
            selftest()
        else:
            sys.exit("usage: c1_eval.py next|halts|probe|pausehours|prevcheck|hits|plan|prepare|verdict|selftest ...")
    except (IndexError, ValueError, OSError) as e:
        print("ERROR %s/%s" % (type(e).__name__, e))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
