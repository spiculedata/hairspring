#!/usr/bin/env python3
# sclib.py : shared library of the N and G specs (byte-identical in both). Pure functions, no files written, no clock.
#   * COLS / parse_rows : the runs.tsv schema written by the drivers (facts.sh fills the fact columns)
#   * classify          : the per-run class of GO-SQLCANCEL-DESIGN section 5.2 (copied; nothing tuned)
#   * wilson / zero_bound / fisher_two_sided / median : the statistics of design section 8
#   * python3 -B sclib.py selftest : exhaustive and anchored tests
import math
import sys

FACT_COLS = ("q scan_ok scan_err fh torn unp ovl fnext nout_other nev nviol nother pf pt hf ovlt errh handt end "
             "vprop vharn vother fatal hpanic hinfra dead").split()
COLS = ("id step arm build mode seed rep rc ms stdout_sha events_sha fp syscalls").split() + FACT_COLS
STR_COLS = {"id", "step", "arm", "build", "mode", "seed", "rep", "stdout_sha", "events_sha", "fp", "syscalls"}

Z = 1.959964                    # 95% two-sided, as design section 8
EXPECTED_EVENTS = 17            # shim build: 1 hello + 7 catalog + 7 assert + 1 work + 1 end (design section 5)
TIMEOUT_RCS = (124, 137)        # the cap fired (timeout, or its -k kill)
HERMIT_RCS = (125, 101)         # Hermit's usual failure status and a Rust panic (design section 5.2)
HARNESS_RCS = (3, 64)           # 3 = a harness invariant failed (design section 2.4); 64 = usage error of the workload
PROP_ID = "go.sql.scan_returns_own_row"
CLASSES = ("NOHIT", "HIT", "TIMEOUT", "HERMIT-FAIL", "CRASH", "HARNESS-FAIL")
RATED = ("NOHIT", "HIT")        # only these enter a rate (design section 8: rated trials)


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


def is_shim(r):
    return r["build"] == "s"


def hit_detail(r):
    """True when the run shows a hit by the per-site rule, whatever else happened (a crash can carry it as detail)."""
    return r["pf"] > 0 if is_shim(r) else r["vprop"] > 0


def report_incomplete(r):
    """The shim build must send its 17 lines and an end line with the exit status; the violation-only build sends no event line."""
    if is_shim(r):
        return r["nev"] != EXPECTED_EVENTS or r["end"] != 0 or r["pf"] < 0 or r["hf"] < 0
    return r["nev"] != 0


def classify(r, hermit):
    """Class of one run. Precedence (design 5.2): timeout is inconclusive; then HERMIT-FAIL, CRASH, HARNESS-FAIL, HIT.
    hermit=False for native runs (no Hermit classes)."""
    rc = r["rc"]
    if rc in TIMEOUT_RCS:
        return "TIMEOUT"
    if hermit and (rc in HERMIT_RCS or r["hpanic"] > 0 or r["hinfra"] > 0):
        return "HERMIT-FAIL"
    if rc not in (0,) + HARNESS_RCS or r["fatal"] > 0 or r["dead"] > 0:
        return "CRASH"
    if (rc in HARNESS_RCS or r["hf"] > 0 or r["vharn"] > 0 or r["vother"] > 0 or r["q"] < 0
            or r["nout_other"] > 0 or report_incomplete(r)):
        return "HARNESS-FAIL"
    return "HIT" if hit_detail(r) else "NOHIT"


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


def fisher_two_sided(a, b, c, d):
    """Two-sided exact p-value of the 2x2 table [[a, b], [c, d]] (sum of the probabilities not above the observed one)."""
    r1, r2, c1, n = a + b, c + d, a + c, a + b + c + d
    if n == 0:
        return 1.0

    def prob(x):
        return math.comb(r1, x) * math.comb(r2, c1 - x) / math.comb(n, c1)

    lo, hi = max(0, c1 - r2), min(r1, c1)
    po = prob(a)
    return min(1.0, sum(prob(x) for x in range(lo, hi + 1) if prob(x) <= po * (1 + 1e-7)))


def median(xs):
    s = sorted(xs)
    if not s:
        return None
    m = len(s) // 2
    return s[m] if len(s) % 2 else (s[m - 1] + s[m]) / 2


def _blank(**kw):
    r = {c: 0 for c in COLS}
    r.update({"id": "x", "step": "s", "arm": "u", "build": "s", "mode": "std", "seed": "1", "rep": "1", "stdout_sha": "-",
              "events_sha": "-", "fp": "-", "syscalls": "-", "q": 4000, "pf": 0, "pt": 10, "hf": 0, "ovlt": 1, "errh": 1,
              "handt": 1, "nev": EXPECTED_EVENTS, "end": 0, "rc": 0, "ms": 10})
    r.update(kw)
    if r["build"] == "v":
        r.update({"nev": 0, "end": -1, "pf": -1, "pt": -1, "hf": -1, "ovlt": -1, "errh": -1, "handt": -1})
        r.update({k: v for k, v in kw.items() if k in ("pf", "hf", "end", "nev")})
    return r


def _oracle(r, hermit):
    """An independent restatement of classify (a flat decision list) for the exhaustive test."""
    rc = r["rc"]
    if rc == 124 or rc == 137:
        return "TIMEOUT"
    if hermit and (rc == 125 or rc == 101 or r["hpanic"] or r["hinfra"]):
        return "HERMIT-FAIL"
    if rc == 1 or rc == 2 or rc >= 128 or rc == 125 or rc == 101 or r["fatal"] or r["dead"] or (rc not in (0, 3, 64)):
        return "CRASH"
    broken = rc in (3, 64) or r["hf"] > 0 or r["vharn"] > 0 or r["vother"] > 0 or r["q"] < 0 or r["nout_other"] > 0
    if r["build"] == "s":
        broken = broken or r["nev"] != 17 or r["end"] != 0 or r["pf"] < 0 or r["hf"] < 0
    else:
        broken = broken or r["nev"] != 0
    if broken:
        return "HARNESS-FAIL"
    hit = (r["pf"] > 0) if r["build"] == "s" else (r["vprop"] > 0)
    return "HIT" if hit else "NOHIT"


def selftest():
    # statistics, anchored to GO-SQLCANCEL-DESIGN section 8.1 (computed there with z = 1.959964)
    assert abs(100 * wilson(0, 2000)[1] - 0.1917) < 5e-5, wilson(0, 2000)
    assert abs(100 * zero_bound(2000) - 0.1497) < 5e-5
    for k, want in ((7, 0.170), (8, 0.203), (10, 0.272), (14, 0.417)):
        assert abs(100 * wilson(k, 2000)[0] - want) < 5e-4, (k, wilson(k, 2000))
    assert abs(100 * wilson(3, 200)[0] - 0.511) < 5e-4 and abs(100 * wilson(0, 200)[1] - 1.885) < 5e-4
    assert wilson(0, 0) == (None, None) and zero_bound(0) is None and median([]) is None
    assert median([3, 1, 2]) == 2 and median([4, 1, 2, 3]) == 2.5
    # Fisher: the classic tea-tasting table, a symmetric table, a brute-force cross-check
    assert abs(fisher_two_sided(3, 1, 1, 3) - 0.4857142857) < 1e-9
    assert abs(fisher_two_sided(5, 5, 5, 5) - 1.0) < 1e-12 and fisher_two_sided(0, 0, 0, 0) == 1.0
    assert fisher_two_sided(0, 200, 0, 200) == 1.0
    a, b, c, d = 3, 197, 0, 200
    tot = [math.comb(200, x) * math.comb(200, 3 - x) / math.comb(400, 3) for x in range(0, 4)]
    brute = sum(p for p in tot if p <= tot[3] * (1 + 1e-7))
    assert abs(fisher_two_sided(a, b, c, d) - brute) < 1e-12
    # classify against the independent oracle over the whole product of the inputs that matter
    n = 0
    for hermit in (False, True):
        for build in ("s", "v"):
            for rc in (0, 1, 2, 3, 64, 101, 124, 125, 137, 139):
                for hp in (0, 1):
                    for hi in (0, 1):
                        for fatal in (0, 1):
                            for dead in (0, 1):
                                for harness in (0, 1):      # a false harness site (shim) or violation line (violation-only)
                                    for hit in (0, 1):
                                        for broken in (0, 1):   # an incomplete report / unparsable stdout
                                            kw = {"rc": rc, "hpanic": hp, "hinfra": hi, "fatal": fatal, "dead": dead}
                                            if build == "s":
                                                kw["hf"] = harness
                                                kw["pf"] = 3 if hit else 0
                                                kw["nev"] = 12 if broken else EXPECTED_EVENTS
                                            else:
                                                kw["vharn"] = harness
                                                kw["vprop"] = 1 if hit else 0
                                                kw["nev"] = 2 if broken else 0
                                            r = _blank(build=build, **kw)
                                            got = classify(r, hermit)
                                            assert got == _oracle(r, hermit), (hermit, build, kw, got, _oracle(r, hermit))
                                            assert got in CLASSES
                                            n += 1
    # anchored cases
    ok = _blank()
    assert classify(ok, False) == "NOHIT" and classify(_blank(pf=3), False) == "HIT"
    assert classify(_blank(build="v", vprop=1), False) == "HIT" and classify(_blank(build="v"), False) == "NOHIT"
    crashed_hit = _blank(rc=2, pf=5, fatal=1)
    assert classify(crashed_hit, True) == "CRASH" and hit_detail(crashed_hit)       # a hit that also crashed is CRASH, hit as detail
    assert classify(_blank(rc=125, pf=5), True) == "HERMIT-FAIL" and classify(_blank(rc=125), False) == "CRASH"
    assert classify(_blank(rc=124, pf=5), True) == "TIMEOUT"
    assert classify(_blank(rc=0, hpanic=1), True) == "HERMIT-FAIL"       # 'panicked at' in stderr, exit 0
    assert classify(_blank(rc=3, hf=2), True) == "HARNESS-FAIL" and classify(_blank(q=-1), False) == "HARNESS-FAIL"
    assert classify(_blank(nev=0, end=-1), False) == "HARNESS-FAIL"      # shim report missing with exit 0
    assert classify(_blank(rc=1), True) == "CRASH"                      # a Hermit deadlock report exits 1; not in the design table, see the specs
    assert classify(_blank(nother=7), True) == "NOHIT"                  # Hermit noise on stderr is not an anomaly
    # parse_rows round trip
    r0 = _blank()
    line = "\t".join(str(r0[c]) for c in COLS)
    assert parse_rows(line + "\n" + "bad\tline\n")[0]["pf"] == 0 and len(parse_rows(line + "\nshort")) == 1
    print("selftest ok: statistics anchored, classify %d combinations" % n)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "selftest":
        selftest()
    else:
        sys.exit("usage: sclib.py selftest")
