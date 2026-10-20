#!/usr/bin/env python3
# x2_eval.py : the pre-registered rules of the X2 spec (section 9). Nothing here is tuned after seeing data.
#   x2_eval.py verdict RUN_DIR   prints the tables and "OUTCOME <code>" (run on the Mac over the archive copy of $R; reads runs.tsv, steps.tsv and dumps.tsv)
#   x2_eval.py selftest          exhaustive and sample tests, no files written (also run on the box as gate EVAL-SELFTEST)
# wilson, zero_bound, pct and rate_text are the functions of GON's sclib.py (same text); the rest is new.
import math
import os
import re
import sys

COLS = ("id cell arm rep rc ms cls subtype prop_false stall_hit hit_also libc_ok rounds signals blocked1 maxb maxstag "
        "ticks spanned nout dump").split()
STR_COLS = {"id", "cell", "arm", "cls", "subtype"}
CELLS = ("NP", "NA")                 # NP: arm P only (its own setting). NA: arms U and C (X1b's setting), one run of each per round
CELL_ARMS = {"NP": ("p",), "NA": ("u", "c")}
Z = 1.959964                         # 95% two-sided (design section 8)
CLASSES = ("NOHIT", "HIT", "STALL", "TIMEOUT", "CRASH", "HARNESS-FAIL")
RATED = ("NOHIT", "HIT", "STALL")    # design 5.2: rated = HIT + NOHIT + STALL
NA_FLOOR = 1000                      # runs per arm in NA below which the native false-fire evidence is too short (design 6.2: D-neg is 1,000 runs)
VACUITY_TICKS = 3                    # mean ticks per run below which a cell reads VACUOUS (design 13c)
HIGH_LOWER = 0.05                    # design 8.4: Wilson lower bound of the hit-run share at least 5%


def wilson(k, n, z=Z):
    """Wilson score interval (low, high) as fractions, or (None, None) when n is 0."""
    if n <= 0:
        return None, None
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def zero_bound(n, alpha=0.05):
    """Exact one-sided upper bound after 0 hits in n trials: 1 - alpha^(1/n) (about 3/n)."""
    return 1 - alpha ** (1.0 / n) if n > 0 else None


def pct(x, digits=4):
    return "n/a" if x is None else ("%." + str(digits) + "f%%") % (100 * x)


def rate_text(k, n):
    lo, hi = wilson(k, n)
    zb = " exact-zero-upper=%s" % pct(zero_bound(n)) if k == 0 and n > 0 else ""
    return "%d/%d wilson95=[%s, %s]%s" % (k, n, pct(lo), pct(hi), zb)


def read_text(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def parse_rows(text):
    """runs.tsv text -> list of dicts. Integer columns become int; a field that is not an integer becomes -1."""
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


def consistent(r):
    """The exit status, the two site flags and the class agree (condlock exits 0, 3 on a property hit, 4 on a stall)."""
    c = r["cls"]
    if c == "NOHIT":
        return r["rc"] == 0 and r["prop_false"] == 0 and r["stall_hit"] == 0
    if c == "HIT":
        return r["rc"] == 3 and r["prop_false"] == 1
    if c == "STALL":
        return r["rc"] == 4 and r["stall_hit"] == 1
    return True


def klass(r):
    """The class of one run as counted. classify.sh's class is taken (per site, never from an engine verdict); a class this native job cannot produce
    (HERMIT-FAIL, DEADLOCK-REPORT, ERR), a wrong libc line, or a class that disagrees with the exit status and the site flags is HARNESS-FAIL."""
    c = r["cls"]
    if c in ("TIMEOUT", "CRASH"):
        return c
    if c not in CLASSES:
        return "HARNESS-FAIL"
    if c in RATED and (r["libc_ok"] != 1 or not consistent(r)):
        return "HARNESS-FAIL"
    return c


def base_subtype(s):
    return s.split("+")[0] if s else "-"


def tables(rows):
    """(cell, arm) -> counts by class, sums, STALL subtype counts."""
    t = {}
    for r in rows:
        if r["cell"] not in CELLS or r["arm"] not in CELL_ARMS[r["cell"]]:
            continue
        c = t.setdefault((r["cell"], r["arm"]), {k: 0 for k in CLASSES})
        k = klass(r)
        c[k] += 1
        c["runs"] = c.get("runs", 0) + 1
        c["ms"] = c.get("ms", 0) + max(r["ms"], 0)
        if r["ticks"] >= 0:
            c["ticks_sum"] = c.get("ticks_sum", 0) + r["ticks"]
            c["ticks_n"] = c.get("ticks_n", 0) + 1
        c["maxstag"] = max(c.get("maxstag", 0), r["maxstag"])
        c["blocked1_sum"] = c.get("blocked1_sum", 0) + max(r["blocked1"], 0)
        c["spanned_sum"] = c.get("spanned_sum", 0) + max(r["spanned"], 0)
        c["hit_also"] = c.get("hit_also", 0) + (1 if r["hit_also"] == 1 and k != "HIT" else 0)
        if k == "STALL":
            sub = c.setdefault("stall_sub", {})
            b = base_subtype(r["subtype"])
            sub[b] = sub.get(b, 0) + 1
    return t


def vacuous(c):
    """A cell whose mean ticks per run is under VACUITY_TICKS is not evidence of 'no false fire' (design 13c)."""
    n = c.get("ticks_n", 0)
    return n > 0 and c.get("ticks_sum", 0) < VACUITY_TICKS * n


def phase_facts(steps_text):
    """gate failure, per-cell phase status, whether a binary changed, whether the driver was signalled or halted."""
    gate, bin_changed, halted = None, False, False
    phases = {c: None for c in CELLS}
    for line in steps_text.splitlines():
        n = line.split("\t", 1)[1] if "\t" in line else line
        m = re.match(r"GATE-FAIL (\S+)", n)
        if m:
            gate = m.group(1)
        m = re.match(r"PHASE (\w+) (DONE|STOPPED)", n)
        if m and m.group(1) in phases:
            phases[m.group(1)] = m.group(2)
        if n.startswith("BIN-CHANGED"):
            bin_changed = True
        if n.startswith("SIGNAL") or n.startswith("HALTED"):
            halted = True
    return gate, phases, bin_changed, halted


def stall_blocks(t):
    """STALL handling (design 8.3, applied natively): a STALL of subtype Q in U or P, with no STALL in C, is reported and changes no code;
    any STALL in C, or any STALL of subtype L, B, M (or one with no subtype) in any arm, blocks claims until understood."""
    for (cell, arm), c in t.items():
        sub = c.get("stall_sub", {})
        if arm == "c" and sum(sub.values()) > 0:
            return True
        if any(b != "Q" for b in sub):
            return True
    return False


def flags(t, phases=None):
    """The booleans the verdict reads."""
    phases = phases or {}
    cnt = lambda arms, key: sum(c.get(key, 0) for (cell, arm), c in t.items() if arm in arms)
    rated = lambda cell, arm: sum(t.get((cell, arm), {}).get(k, 0) for k in RATED)
    done = [c for c in CELLS if phases.get(c) == "DONE"]
    na_runs = [t.get(("NA", a), {}).get("runs", 0) for a in ("u", "c")]
    return {
        "harness": cnt(("u", "c", "p"), "HARNESS-FAIL") > 0,
        "crash": cnt(("u", "c", "p"), "CRASH") > 0,
        "control_hit": cnt(("c",), "HIT") > 0,
        "stall_block": stall_blocks(t),
        "norated": any(rated(c, a) == 0 for c in done for a in CELL_ARMS[c]),
        "na_short": phases.get("NA") == "DONE" and min(na_runs) < NA_FLOOR,
        "nu_hit": t.get(("NA", "u"), {}).get("HIT", 0) > 0,
    }


def n_verdict(gate, bin_changed, phases, fl):
    """First rule that holds. Always returns an N- code. Data-based blocks come before 'not complete', so a control hit in a partial run is still reported."""
    if gate:
        return "N-NOT-STARTED/%s" % gate
    if bin_changed:
        return "N-INVALID/BIN-CHANGED"
    if fl["harness"]:
        return "N-HARNESS"
    if fl["crash"]:
        return "N-CRASH"
    if fl["control_hit"]:
        return "N-STOP-CONTROL-HIT"
    if fl["stall_block"]:
        return "N-STALL"
    for c in CELLS:
        if phases.get(c) != "DONE":
            return "N-INCONCLUSIVE/%s-%s" % (c, phases.get(c) or "NOT-RUN")
    if fl["na_short"]:
        return "N-INCONCLUSIVE/NA-SHORT"
    if fl["norated"]:
        return "N-INCONCLUSIVE/NO-RATED-RUNS"
    return "N-DONE/NU-HIT" if fl["nu_hit"] else "N-DONE/NU-ZERO"


def suffixes(t):
    """+VACUOUS-<arm> for every cell/arm with runs whose mean ticks per run is under VACUITY_TICKS."""
    return "".join("+VACUOUS-%s" % arm.upper() for (cell, arm), c in sorted(t.items(), key=lambda kv: (kv[0][1], kv[0][0])) if vacuous(c))


def high_rule(k, n, c_hits):
    """Design 8.4, exactly: N-u is 'high' when the Wilson 95% lower bound of its hit-run share in cell NA is at least 5% with N-c = 0 hits in NA.
    Per run, presentation only. Returns (is_high, lower bound)."""
    lo = wilson(k, n)[0]
    return (lo is not None and lo >= HIGH_LOWER and c_hits == 0), lo


# ---- the state dump of a hit or stall (the line `condlock: HIT|STALL subtype=... cond=U wseq=... g_refs=a,b g_size=a,b ... g_signals=a,b`)
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
    """Pre-registered shape tally, reported and never gating. DP-SHAPE (the 'unconsumed signal for an empty group' shape): layout U (arms U and P), lock free, at least
    one waiter, and for one group index g (h = the other): g_signals[g] = 2, g_size[g] = 0, g_refs[g] = 0 and g_refs[h] = 2 x waiting.
    Also returns whether that g is the dump's own g1 (a secondary description). DP-OTHER: a parsable layout-U dump that does not have the shape.
    DP-NA-LAYOUT: layout C (no g_refs) or no condvar fields. DP-UNPARSABLE: anything else."""
    d = parse_dump(line)
    if d is None:
        return "DP-UNPARSABLE", False
    if d.get("cond") in ("C", "none"):
        return "DP-NA-LAYOUT", False
    if d.get("cond") != "U":
        return "DP-UNPARSABLE", False
    try:
        waiting, locked, g1 = int(d["waiting"]), int(d["locked"]), int(d["g1"])
    except (KeyError, ValueError):
        return "DP-UNPARSABLE", False
    refs, size, sig = pair(d.get("g_refs")), pair(d.get("g_size")), pair(d.get("g_signals"))
    if refs is None or size is None or sig is None or g1 not in (0, 1):
        return "DP-UNPARSABLE", False
    if locked == 0 and waiting >= 1:
        for g in (0, 1):
            h = 1 - g
            if sig[g] == 2 and size[g] == 0 and refs[g] == 0 and refs[h] == 2 * waiting:
                return "DP-SHAPE", g == g1
    return "DP-OTHER", False


def dump_tally(dump_rows, rows):
    """arm -> counts. dump_rows: (id, arm, class, line). Also the HIT/STALL runs that have no dump line."""
    tally = {}
    for _id, arm, cls, line in dump_rows:
        c = tally.setdefault(arm, {"DP-SHAPE": 0, "DP-SHAPE-G1": 0, "DP-OTHER": 0, "DP-NA-LAYOUT": 0, "DP-UNPARSABLE": 0, "dumps": 0})
        lab, isg1 = dump_pattern(line)
        c[lab] += 1
        c["dumps"] += 1
        if lab == "DP-SHAPE" and isg1:
            c["DP-SHAPE-G1"] += 1
    have = {d[0] for d in dump_rows}
    missing = {}
    for r in rows:
        if r["cls"] in ("HIT", "STALL") and r["dump"] != 1 and r["id"] not in have:
            missing[r["arm"]] = missing.get(r["arm"], 0) + 1
    return tally, missing


def parse_dumps(text):
    out = []
    for line in text.splitlines():
        f = line.split("\t", 3)
        if len(f) == 4:
            out.append(tuple(f))
    return out


def line_for(t, cell, arm):
    c = t.get((cell, arm))
    if not c:
        return "%s %s: no runs" % (cell, arm)
    rated = sum(c[k] for k in RATED)
    other = "timeout=%d crash=%d harness=%d stall=%d" % (c["TIMEOUT"], c["CRASH"], c["HARNESS-FAIL"], c["STALL"])
    mt = "n/a" if not c.get("ticks_n") else "%.2f" % (c["ticks_sum"] / c["ticks_n"])
    return "%s %s: runs=%d rated=%d hit runs=%s | %s | mean_ticks=%s%s maxstag=%d | run_seconds=%.1f | blocked1_sum=%d spanned_sum=%d" % (
        cell, arm, c["runs"], rated, rate_text(c["HIT"], rated), other, mt, " VACUOUS" if vacuous(c) else "", c.get("maxstag", 0),
        c["ms"] / 1000.0, c["blocked1_sum"], c["spanned_sum"])


def main_verdict(run_dir):
    rows = parse_rows(read_text(os.path.join(run_dir, "runs.tsv")))
    gate, phases, bin_changed, halted = phase_facts(read_text(os.path.join(run_dir, "steps.tsv")))
    dump_rows = parse_dumps(read_text(os.path.join(run_dir, "dumps.tsv")))
    t = tables(rows)
    fl = flags(t, phases)
    print("gate     :", gate or "none")
    print("phases   :", phases, "| binary changed:", bin_changed, "| signalled or halted:", halted)
    for cell in CELLS:
        for arm in CELL_ARMS[cell]:
            print(line_for(t, cell, arm))
    print("recorded, not rules:")
    for (cell, arm), c in sorted(t.items()):
        print("  %s %s: STALL by subtype %s | hits that fell in an earlier class (hit_also) = %d" % (cell, arm, dict(sorted(c.get("stall_sub", {}).items())) or "none", c["hit_also"]))
    na_u, na_c = t.get(("NA", "u"), {}), t.get(("NA", "c"), {})
    nc = sum(na_c.get(k, 0) for k in RATED)
    print("D-NEG (arm C, cell NA): runs=%d rated=%d hits=%d stalls=%d mean_ticks=%s -> %s" % (
        na_c.get("runs", 0), nc, na_c.get("HIT", 0), na_c.get("STALL", 0), "n/a" if not na_c.get("ticks_n") else "%.2f" % (na_c["ticks_sum"] / na_c["ticks_n"]),
        "VACUOUS: not evidence of 'no false fire'" if vacuous(na_c) else ("rate " + rate_text(na_c.get("HIT", 0), nc) if nc else "no rated runs")))
    nu = sum(na_u.get(k, 0) for k in RATED)
    is_high, lo = high_rule(na_u.get("HIT", 0), nu, na_c.get("HIT", 0))
    print("HIGH-RULE (design 8.4, presentation only): NA arm U hit-run share %s; Wilson lower bound %s; arm C hits in NA = %d -> nu_high=%s" % (
        rate_text(na_u.get("HIT", 0), nu), pct(lo), na_c.get("HIT", 0), "yes" if is_high else "no"))
    if is_high:
        print("HIGH-RULE consequence: R-HIT-NATIVE-EASY presentation applies only if the Hermit C1 outcome is R-HIT (exact replay leads the page); nothing else changes")
    tally, missing = dump_tally(dump_rows, rows)
    for arm in ("u", "c", "p"):
        c = tally.get(arm)
        if c:
            print("DUMP-TALLY arm %s: dumps=%d DP-SHAPE=%d (of which g is the dump's g1: %d) DP-OTHER=%d DP-NA-LAYOUT=%d DP-UNPARSABLE=%d | HIT/STALL runs without a dump line=%d" % (
                arm, c["dumps"], c["DP-SHAPE"], c["DP-SHAPE-G1"], c["DP-OTHER"], c["DP-NA-LAYOUT"], c["DP-UNPARSABLE"], missing.get(arm, 0)))
        else:
            print("DUMP-TALLY arm %s: no dumps | HIT/STALL runs without a dump line=%d" % (arm, missing.get(arm, 0)))
    pu = t.get(("NP", "p"), {})
    print("  prediction check (an inference, never a rule): NA arm U hit runs=%s of %s rated; NA arm C hit runs=%s; NP arm P hit runs=%s of %s rated" % (
        na_u.get("HIT"), nu, na_c.get("HIT"), pu.get("HIT"), sum(pu.get(k, 0) for k in RATED)))
    print("OUTCOME", n_verdict(gate, bin_changed, phases, fl) + suffixes(t))


def _row(**kw):
    r = {c: 0 for c in COLS}
    r.update({"id": "x", "cell": "NA", "arm": "u", "cls": "NOHIT", "subtype": "-", "rc": 0, "ms": 1000, "prop_false": 0, "stall_hit": 0, "hit_also": 0,
              "libc_ok": 1, "nout": 1, "rounds": 40000, "signals": 40000, "ticks": 18, "dump": 0})
    r.update(kw)
    return r


U_DUMP = ("condlock: HIT subtype=Q+attributed s_seq=38324 blocked=4 waiting=4 locked=0 progress=76648 stagnant=10 ticks=17 cond=U wseq=58771 "
          "g1_start=58759 g1=0 g_refs=0,8 g_size=0,0 g1_orig_size=8 wrefs>>3=4 wrefs_match=1 g_signals=2,0")


def selftest():
    # statistics: the same anchors as GON's sclib.py (design section 8.1, z = 1.959964)
    assert abs(100 * wilson(0, 2000)[1] - 0.1917) < 5e-5, wilson(0, 2000)
    assert abs(100 * zero_bound(2000) - 0.1497) < 5e-5
    assert abs(100 * zero_bound(1000) - 0.2988) < 5e-4
    for k, want in ((7, 0.170), (8, 0.203), (10, 0.272), (14, 0.417)):
        assert abs(100 * wilson(k, 2000)[0] - want) < 5e-4, (k, wilson(k, 2000))
    assert wilson(0, 0) == (None, None) and zero_bound(0) is None
    # the 'high' rule: the line is a Wilson lower bound of 5% with N-c = 0; monotone in k, and a control hit switches it off
    n = 2000
    ks = [k for k in range(0, n + 1) if high_rule(k, n, 0)[0]]
    assert ks and ks == list(range(ks[0], n + 1)) and not high_rule(ks[0] - 1, n, 0)[0], ks[:3]
    assert wilson(ks[0], n)[0] >= HIGH_LOWER > wilson(ks[0] - 1, n)[0]
    assert not high_rule(n, n, 1)[0] and not high_rule(0, 0, 0)[0] and high_rule(n, n, 0)[0]
    # klass: classify.sh's class is taken; anything this native job cannot produce, or inconsistent, is HARNESS-FAIL
    assert klass(_row()) == "NOHIT" and klass(_row(cls="HIT", rc=3, prop_false=1)) == "HIT" and klass(_row(cls="STALL", rc=4, stall_hit=1, subtype="Q")) == "STALL"
    assert klass(_row(cls="HIT", rc=0, prop_false=1)) == "HARNESS-FAIL" and klass(_row(cls="HIT", rc=3, prop_false=0)) == "HARNESS-FAIL"
    assert klass(_row(cls="STALL", rc=3, stall_hit=1)) == "HARNESS-FAIL" and klass(_row(cls="NOHIT", rc=0, stall_hit=1)) == "HARNESS-FAIL"
    assert klass(_row(libc_ok=0)) == "HARNESS-FAIL" and klass(_row(cls="TIMEOUT", rc=124, libc_ok=0)) == "TIMEOUT"
    assert klass(_row(cls="CRASH", rc=139)) == "CRASH" and klass(_row(cls="HERMIT-FAIL")) == "HARNESS-FAIL" and klass(_row(cls="DEADLOCK-REPORT")) == "HARNESS-FAIL"
    assert klass(_row(cls="ERR")) == "HARNESS-FAIL" and klass(_row(cls="HARNESS-FAIL", rc=5)) == "HARNESS-FAIL"
    # tables, vacuity, STALL handling
    rows = ([_row(arm="u", cls="HIT", rc=3, prop_false=1, subtype="Q+attributed", ticks=12, dump=1)] * 3 + [_row(arm="u")] * 7 + [_row(arm="c")] * 10
            + [_row(cell="NP", arm="p", ticks=1)] * 4 + [_row(cell="NA", arm="c", cls="TIMEOUT", rc=124, ticks=-1)] + [_row(cell="XX")] + [_row(cell="NP", arm="u")])
    t = tables(rows)
    assert t[("NA", "u")]["HIT"] == 3 and t[("NA", "u")]["NOHIT"] == 7 and t[("NA", "u")]["runs"] == 10 and t[("NA", "c")]["TIMEOUT"] == 1
    assert ("XX", "u") not in t and ("NP", "u") not in t and t[("NP", "p")]["runs"] == 4
    assert vacuous(t[("NP", "p")]) and not vacuous(t[("NA", "c")]) and not vacuous({}) and not vacuous({"ticks_sum": 6, "ticks_n": 2}) and vacuous({"ticks_sum": 5, "ticks_n": 2})
    assert suffixes(t) == "+VACUOUS-P"
    q = tables([_row(cls="STALL", rc=4, stall_hit=1, subtype="Q")])
    assert not stall_blocks(q) and q[("NA", "u")]["stall_sub"] == {"Q": 1}
    assert stall_blocks(tables([_row(arm="c", cls="STALL", rc=4, stall_hit=1, subtype="Q")]))
    for sub in ("L", "B", "M", "NONE", "-"):
        assert stall_blocks(tables([_row(cls="STALL", rc=4, stall_hit=1, subtype=sub)])), sub
        assert stall_blocks(tables([_row(cell="NP", arm="p", cls="STALL", rc=4, stall_hit=1, subtype=sub)])), sub
    assert not stall_blocks(tables([_row(cell="NP", arm="p", cls="STALL", rc=4, stall_hit=1, subtype="Q")]))
    # flags and the verdict: every combination of its inputs
    states = (None, "DONE", "STOPPED")
    n = 0
    for gate in (None, "BUSY"):
        for bc in (False, True):
            for p1 in states:
                for p2 in states:
                    for h in (False, True):
                        for cr in (False, True):
                            for ch in (False, True):
                                for sb in (False, True):
                                    for nr in (False, True):
                                        for sh in (False, True):
                                            for nu in (False, True):
                                                fl = {"harness": h, "crash": cr, "control_hit": ch, "stall_block": sb, "norated": nr, "na_short": sh, "nu_hit": nu}
                                                v = n_verdict(gate, bc, {"NP": p1, "NA": p2}, fl)
                                                n += 1
                                                assert v.startswith("N-"), v
                                                clean = gate is None and not bc and not (h or cr or ch or sb or nr or sh) and p1 == p2 == "DONE"
                                                assert v.startswith("N-DONE/") == clean, (gate, bc, p1, p2, fl, v)
                                                if clean:
                                                    assert v == ("N-DONE/NU-HIT" if nu else "N-DONE/NU-ZERO")
                                                if gate:
                                                    assert v == "N-NOT-STARTED/BUSY"
    ph = {"NP": "DONE", "NA": "DONE"}
    allf = {"harness": True, "crash": True, "control_hit": True, "stall_block": True, "norated": True, "na_short": True, "nu_hit": True}
    none = {k: False for k in allf}
    assert n_verdict(None, True, ph, allf) == "N-INVALID/BIN-CHANGED"
    assert n_verdict(None, False, ph, allf) == "N-HARNESS"
    assert n_verdict(None, False, ph, {**allf, "harness": False}) == "N-CRASH"
    assert n_verdict(None, False, ph, {**allf, "harness": False, "crash": False}) == "N-STOP-CONTROL-HIT"
    assert n_verdict(None, False, ph, {**allf, "harness": False, "crash": False, "control_hit": False}) == "N-STALL"
    assert n_verdict(None, False, ph, {**none, "na_short": True, "norated": True, "nu_hit": True}) == "N-INCONCLUSIVE/NA-SHORT"
    assert n_verdict(None, False, ph, {**none, "norated": True}) == "N-INCONCLUSIVE/NO-RATED-RUNS"
    assert n_verdict(None, False, {"NP": "DONE", "NA": None}, {**allf, "harness": False, "crash": False, "control_hit": False, "stall_block": False}) == "N-INCONCLUSIVE/NA-NOT-RUN"
    assert n_verdict(None, False, {"NP": "STOPPED", "NA": None}, none) == "N-INCONCLUSIVE/NP-STOPPED"
    # flags from rows: na_short needs NA DONE; a P hit and a U STALL-Q change no code
    mk = lambda cell, arm, k, **kw: [_row(cell=cell, arm=arm, **kw)] * k
    base = mk("NA", "u", NA_FLOOR) + mk("NA", "c", NA_FLOOR) + mk("NP", "p", 5)
    fl = flags(tables(base), {"NP": "DONE", "NA": "DONE"})
    assert fl == {"harness": False, "crash": False, "control_hit": False, "stall_block": False, "norated": False, "na_short": False, "nu_hit": False}, fl
    assert flags(tables(base[1:]), {"NP": "DONE", "NA": "DONE"})["na_short"] and not flags(tables(base[1:]), {"NP": "DONE", "NA": None})["na_short"]
    withp = base + mk("NP", "p", 3, cls="HIT", rc=3, prop_false=1) + mk("NA", "u", 2, cls="STALL", rc=4, stall_hit=1, subtype="Q")
    flp = flags(tables(withp), {"NP": "DONE", "NA": "DONE"})
    assert not flp["control_hit"] and not flp["stall_block"] and n_verdict(None, False, ph, flp) == "N-DONE/NU-ZERO"
    assert flags(tables(base + mk("NA", "c", 1, cls="HIT", rc=3, prop_false=1)), ph)["control_hit"]
    assert flags(tables([_row(arm="u")]), {"NA": "DONE"})["norated"] and flags(tables([_row(arm="u")]), {"NA": None})["norated"] is False
    # the state dump shape tally
    assert dump_pattern(U_DUMP) == ("DP-SHAPE", True)
    other_g = U_DUMP.replace("g_refs=0,8", "g_refs=8,0").replace("g_signals=2,0", "g_signals=0,2").replace("g_size=0,0", "g_size=0,0")
    assert dump_pattern(other_g) == ("DP-SHAPE", False), dump_pattern(other_g)         # the mirror image: g = 1, the dump's g1 is 0
    for old, new in (("locked=0", "locked=1"), ("waiting=4", "waiting=3"), ("g_refs=0,8", "g_refs=1,7"), ("g_refs=0,8", "g_refs=0,4"), ("g_signals=2,0", "g_signals=4,0"),
                     ("g_signals=2,0", "g_signals=0,0"), ("g_size=0,0", "g_size=1,0"), ("waiting=4", "waiting=0")):
        assert dump_pattern(U_DUMP.replace(old, new))[0] == "DP-OTHER", (old, new)
    assert dump_pattern(U_DUMP.replace("cond=U", "cond=C"))[0] == "DP-NA-LAYOUT" and dump_pattern(U_DUMP.replace("cond=U", "cond=none"))[0] == "DP-NA-LAYOUT"
    assert dump_pattern(U_DUMP.replace("cond=U", "cond=Z"))[0] == "DP-UNPARSABLE" and dump_pattern("garbage")[0] == "DP-UNPARSABLE"
    assert dump_pattern(U_DUMP.replace("g_refs=0,8", "g_refs=x"))[0] == "DP-UNPARSABLE" and dump_pattern(U_DUMP.replace(" waiting=4", ""))[0] == "DP-UNPARSABLE"
    assert dump_pattern(U_DUMP.replace("g1=0", "g1=7"))[0] == "DP-UNPARSABLE"
    assert dump_pattern(U_DUMP.replace("condlock: HIT", "condlock: STALL").replace("Q+attributed", "Q"))[0] == "DP-SHAPE"
    tally, missing = dump_tally([("a", "u", "HIT", U_DUMP), ("b", "u", "HIT", "garbage")], [_row(id="a", cls="HIT", arm="u", dump=1), _row(id="c", cls="HIT", arm="p", dump=0)])
    assert tally["u"]["DP-SHAPE"] == 1 and tally["u"]["DP-UNPARSABLE"] == 1 and tally["u"]["DP-SHAPE-G1"] == 1 and missing == {"p": 1}, (tally, missing)
    # parse_rows and phase_facts
    r0 = _row()
    line = "\t".join(str(r0[c]) for c in COLS)
    assert parse_rows(line + "\n" + "bad\tline\n")[0]["ticks"] == 18 and len(parse_rows(line + "\nshort")) == 1
    assert parse_rows(line.replace("\t18\t", "\tx\t", 1))[0]["ticks"] == -1
    g, p, b, hlt = phase_facts("1\tGATE-FAIL CORES\n2\tPHASE NP DONE runs=200\n3\tPHASE NA STOPPED at round 5\n4\tBIN-CHANGED condlock-U\n5\tSIGNAL: x\n")
    assert g == "CORES" and p == {"NP": "DONE", "NA": "STOPPED"} and b and hlt
    print("selftest ok: statistics anchored, high rule, classes, dump shapes")
    print("selftest ok: %d verdict combinations" % n)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "verdict" and len(sys.argv) > 2:
        main_verdict(sys.argv[2])
    elif mode == "selftest":
        selftest()
    else:
        sys.exit("usage: x2_eval.py verdict RUN_DIR | selftest")
