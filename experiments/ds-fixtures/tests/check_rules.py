"""Grep-style checks of the fixture rules, applied to every C, Go and Rust source of the DS fixtures.

Rules (each is a regular expression over the source with comments removed):
  timeouts   no -1 timeout to epoll_wait or poll, no SO_RCVTIMEO (nor the Rust or Go helpers that set it)
  names      no name lookups: numeric 127.0.0.1 only
  random     no randomness: no rand, getrandom, math/rand, RandomState, no hash maps or sets whose iteration could
             reach output (tables are arrays)
  oneline    no buffered stdout output: lines go out through one write(2) helper
  launcher   dsl never unshares, never calls setsid, never clones with namespace flags

The checker first proves each rule fires on a bad sample, then scans the real sources. Exit status 1 on any hit.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

RULES = [
    ("timeouts", r"epoll_wait\s*\([^;]*,\s*-1\s*\)"),
    ("timeouts", r"(?<![A-Za-z_])poll\s*\([^;]*,\s*-1\s*\)"),
    ("timeouts", r"\bppoll\b|\bINFTIM\b"),
    ("timeouts", r"SO_RCVTIMEO|set_read_timeout|set_write_timeout"),
    ("names", r"getaddrinfo|gethostbyname|getnameinfo|\"localhost\"|LookupHost|LookupIP|LookupAddr|net\.Resolver"),
    ("names", r"to_socket_addrs|ToSocketAddrs"),
    ("random", r"(?<![A-Za-z_])s?rand\s*\(|\brandom\s*\(|getrandom|arc4random|/dev/u?random"),
    ("random", r"math/rand|crypto/rand|RandomState|\bHashMap\b|\bHashSet\b|\brand::"),
    ("oneline", r"(?<![A-Za-z_])(printf|puts|putchar|fputs)\s*\(|fprintf\s*\(\s*stdout"),
    ("oneline", r"fmt\.Print|(?<![A-Za-z_])println!|(?<![A-Za-z_])print!"),
]
LAUNCHER_RULES = [("launcher", r"\bunshare\b|\bsetsid\b|CLONE_NEW")]

SAMPLES = {
    "timeouts": ["epoll_wait(ep, ev, 8, -1)", "poll(p, 1, -1)", "ppoll(a)", "x SO_RCVTIMEO y", "s.set_read_timeout(t)"],
    "names": ["getaddrinfo(h)", 'dial("localhost")', "net.LookupHost(x)", "addr.to_socket_addrs()"],
    "random": ["x = rand();", "getrandom(b, 8)", 'import "math/rand"', "let m: HashMap<u8,u8>", "srand(1)"],
    "oneline": ['printf("x")', 'fmt.Println("x")', 'println!("x")', "puts(s)"],
    "launcher": ["unshare(CLONE_NEWNS)", "setsid()"],
}


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def scan(text, rules):
    stripped = strip_comments(text)
    return [(name, m.group(0)) for name, pattern in rules for m in re.finditer(pattern, stripped)]


def sources():
    out = []
    for base, _dirs, files in os.walk(ROOT):
        if any(skip in base for skip in ("/build", "/target", "/tests", "/.git")):
            continue
        for f in files:
            if f.endswith((".c", ".h", ".go", ".rs")) and not f.endswith("_test.go"):
                out.append(os.path.join(base, f))
    return sorted(out)


def prove_rules_fire():
    all_rules = RULES + LAUNCHER_RULES
    for rule, samples in SAMPLES.items():
        for s in samples:
            hits = [n for n, _ in scan(s, all_rules)]
            if rule not in hits:
                sys.exit("checker self-test failed: rule %r did not fire on %r" % (rule, s))
    good = "int n = snprintf(b, 4, \"x\"); ds_emit(\"x\"); epoll_wait(e, v, 8, timeout_ms); eprintln!(\"x\"); // rand( getrandom\n"
    if scan(good, all_rules):
        sys.exit("checker self-test failed: a clean sample was flagged: %r" % scan(good, all_rules))


def main():
    prove_rules_fire()
    files = sources()
    if not files:
        sys.exit("no sources found")
    bad = 0
    for path in files:
        rules = RULES + (LAUNCHER_RULES if os.path.basename(path) == "dsl.c" else [])
        with open(path) as f:
            for rule, text in scan(f.read(), rules):
                bad += 1
                print("RULE %s: %s: %r" % (rule, os.path.relpath(path, ROOT), text))
    # The readiness wrapper must raise a negative timeout to zero, so no caller can pass an infinite wait.
    with open(os.path.join(ROOT, "c", "ds_common.h")) as f:
        if "if (timeout_ms < 0) timeout_ms = 0;" not in f.read():
            bad += 1
            print("RULE timeouts: ds_common.h no longer clamps a negative readiness timeout")
    print("checked %d source files, %d violations" % (len(files), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
