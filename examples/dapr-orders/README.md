# Dapr orders example

A consumer that must apply each order exactly once, tested against a simulated Dapr sidecar.

| File | Purpose |
|---|---|
| `buggy_app.py` | Deduplicates with an in-memory set: breaks when the app restarts between a delivery and its redelivery |
| `fixed_app.py` | Deduplicates with a first-write-wins marker in the state store: survives restarts |
| `scenario.toml` | Workload and fault plan for `anthill-dapr-sim` |

Run it directly (no Hermit, so faults are seeded but thread timing is not):

    anthill-dapr-sim --scenario scenario.toml --seed 3 -- python3 buggy_app.py

Or let Anthill explore seeds under Hermit (Linux x86-64). In `anthill.toml`:

    [target.orders-buggy]
    kind = "process"
    cwd = "/path/to/examples/dapr-orders"
    command = ["/path/to/anthill-dapr-sim", "--scenario", "scenario.toml", "--", "/usr/bin/python3", "buggy_app.py"]
    [target.orders-buggy.budget]
    max_trials = 40

then `anthill test orders-buggy`. The Hermit backend sets `ANTHILL_SEED` to each trial seed, which the
simulator uses to draw its faults.
