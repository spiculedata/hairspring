# hairspring (Python)

In-source property assertions for Python programs under test. A call site updates counters in memory; the program
reports once, at exit, in [guest protocol v1](../../docs/guest-protocol.md). Pure Python, standard library only,
CPython 3.12 or newer, Linux and macOS.

```python
import hairspring as hs

hs.always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance)
hs.sometimes(waited, "bank.writer.waited", "a writer waited for the lock")
hs.reachable("bank.recovery.ran", "recovery ran")
hs.work(1)
hs.thread("writer", 0)
```

**A site that never runs is invisible, not unmet.** A site is catalogued when it first runs; call
`hs.declare(hs.Kind.REACHABLE, id, msg)` once, at import time, for every site whose absence matters, or a run that never
reaches it cannot report it as unmet.

**Evaluations made in a forked child are lost** (`os.fork()`, and `multiprocessing` with the `fork` start method, the
default on Linux for Python 3.12 and 3.13): only the process that started the SDK reports, and nothing is counted in
`end.dropped` for the child. Use `multiprocessing.get_context("spawn")` for workers that assert.

* Documentation: [docs/python-sdk.md](../../docs/python-sdk.md) (API, environment, processes and threads, thread safety,
  the planted race, evidence, limits).
* Use it from the source tree with `PYTHONPATH=sdks/python/src`, or `pip install ./sdks/python`.
* Example: `PYTHONPATH=sdks/python/src python3 sdks/python/examples/quickstart.py`.
* Tests: `python3 -m unittest discover -s sdks/python/tests -t sdks/python` from the repository root; conformance against
  the reference parser: `cargo test -p hairspring-props --test python_sdk` (set `HS_REQUIRE_PYTHON=1` to make a missing
  Python an error instead of a skip).
* `samples/hs_sample.py` holds the programs the tests run; the planted race and its locked twin are in
  [`examples/language-fixtures/python`](../../examples/language-fixtures/python).
