# Dapr orders example

A consumer that must apply each order exactly once, tested against a simulated Dapr sidecar.

| File | Purpose |
|---|---|
| `buggy_app.py` | Deduplicates with an in-memory set: breaks when the app restarts between a delivery and its redelivery |
| `fixed_app.py` | Deduplicates with a first-write-wins marker in the state store: survives restarts |
| `scenario.toml` | Workload and fault plan for `hairspring-dapr-sim` |

Run it directly (no Hermit, so faults are seeded but thread timing is not):

    hairspring-dapr-sim --scenario scenario.toml --seed 3 -- python3 buggy_app.py

Or let Hairspring explore seeds under Hermit (Linux x86-64). In `hairspring.toml`:

    [target.orders-buggy]
    kind = "process"
    cwd = "/path/to/examples/dapr-orders"
    command = ["/path/to/hairspring-dapr-sim", "--scenario", "scenario.toml", "--", "/usr/bin/python3", "buggy_app.py"]
    [target.orders-buggy.budget]
    max_trials = 40

then `hairspring test orders-buggy`. The Hermit backend sets `HAIRSPRING_SEED` to each trial seed, which the
simulator uses to draw its faults.
