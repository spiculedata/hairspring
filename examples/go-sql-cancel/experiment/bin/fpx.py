#!/usr/bin/env python3
# fpx.py SUMMARY.json : prints "<fp16> <syscalls>", or "NOSUMMARY NA".
# <fp16> is computed EXACTLY as F0's fp.py does (sha256 over the five deterministic fields of Hermit's --summary-json, compact sorted-key JSON joined by '|',
# first 16 hex; the same five fields as crates/hairspring-hermit/src/summary.rs, hashed with sha256 instead of the engine's FNV, so only equality is meaningful).
# <syscalls> is the summary's own 'syscalls' count (design 7.4 G2b), or NA.
import hashlib
import json
import sys

try:
    v = json.load(open(sys.argv[1]))
    keys = ["sched_turns", "syscalls", "virttime_elapsed", "timeslice_stats", "per_thread_timeslice"]
    canon = "|".join(json.dumps(v.get(k), sort_keys=True, separators=(",", ":")) for k in keys)
    sc = v.get("syscalls")
    print(hashlib.sha256(canon.encode()).hexdigest()[:16], sc if isinstance(sc, int) and not isinstance(sc, bool) else "NA")
except Exception:
    print("NOSUMMARY NA")
