<h1 align="center">Anthill</h1>
<p align="center"><strong>Deterministic simulation testing you can run on your laptop and your own servers.</strong></p>
<p align="center">
  <img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue">
  <img alt="Status: experimental" src="https://img.shields.io/badge/status-experimental-orange">
  <img alt="Platforms: macOS and Linux" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux-lightgrey">
</p>

Anthill explores many executions of a system, finds the rare interleaving or fault sequence that breaks a
property, **shrinks it to a minimal trace, and hands you a bundle that replays it exactly**. It is inspired by
the deterministic-simulation testing used by FoundationDB, TigerBeetle and Antithesis, built to be open,
self-hostable, and usable without a cloud service.

<p align="center"><img src="docs/images/failure-view.png" alt="A minimal 8-step reproduction of a duplicate-delivery bug, with Replay and state inspection" width="760"></p>

```text
$ anthill test pubsub-buggy --seed 5
anthill: exploring pubsub-buggy (seed 5)
FAILED  pubsub-buggy (sim)
  invariant violated; shrunk from 23 to 8 steps
  steps (8):
      1. Deliver(1, 0)
      2. Tick
      3. Tick
      4. Deliver(0, 0)
      5. Step(1)
      6. Step(0)
      7. Step(1)
      8. Step(0)
  bundle written: .anthill/bundles/pubsub-buggy-d3858727.anthill  (replay with `anthill replay .anthill/bundles/pubsub-buggy-d3858727.anthill`)
$ anthill replay .anthill/bundles/pubsub-buggy-d3858727.anthill   # exits 1 while the bug exists
```

## What you get

| | |
|---|---|
| **Find** | Seeded exploration with snapshot-and-branch (about 3.5x fewer steps than restarting from scratch on the demo targets; your system will differ) |
| **Shrink** | Delta-debugging to a 1-minimal trace that preserves the *same* failure |
| **Replay** | One-file text bundles; replay is exact, checked against the program version |
| **Check determinism** | `anthill doctor` runs the same seed twice and reports the first divergence |
| **Use it from anywhere** | CLI, REST + live events, web UI, **MCP server for AI agents**, GitHub Action, Claude Code plugin |
| **Scale out** | SQLite locally; PostgreSQL + remote workers for teams; Docker Compose included |

## Backends: how much determinism do you get?

Determinism is a property of how a system is run, and it is hard. Anthill is explicit about what each backend
gives you and where it runs. We measured these (see [docs/determinism.md](docs/determinism.md)).

| Backend | You provide | Determinism | macOS | Linux x86-64 | Status |
|---|---|---|---|---|---|
| **`sim`** | A state machine implementing the `Sim` trait (Rust) | Total: the state is plain data | Yes | Yes | Stable core |
| **`wasm`** | A WebAssembly module using `anthill.choose/rand/fail` imports | Total for the guest (NaN canonicalisation, fuel, memory limits); snapshot/fork of memory | Yes | Yes | Working |
| **`process`** | An unmodified Linux program (threads, sockets) | Exploration by scheduler seed under [Hermit](https://github.com/rrnewton/hermit); failures reproduce exactly | **No** | **Yes** (bare metal or PMU-capable VM) | Experimental |

Measured on real hardware: a WASM trace hash recorded on macOS arm64 is identical on Linux x86-64; the
Hermit backend found a real check-then-act threading bug in a C program, reproduced it 3/3 times, and
found a duplicate-delivery bug in an unmodified Python Dapr consumer through a simulated sidecar.

What we do **not** claim: bit-exact replay of arbitrary programs on macOS (there is no supported way to do
this), a general-purpose deterministic hypervisor like Antithesis's, or that Hermit (a young open-source
project) is production-hardened. See [Limitations](#limitations).

## Quick start

```bash
git clone <this repo> && cd anthill
make ui                       # build the web UI (embedded into the server binary)
cargo build --release -p anthill-cli
export PATH="$PWD/target/release:$PATH"

anthill targets               # built-in demos
anthill test lock-lease --seed 7
anthill doctor lock-lease     # is the target deterministic?
anthill serve                 # REST API, live UI and MCP at http://127.0.0.1:7878
```

Without `make ui` the server still works (API and MCP); it shows a placeholder page instead of the UI.

### Test your own system

Write a state machine (see [`crates/anthill-core/src/sim.rs`](crates/anthill-core/src/sim.rs) and the worked
example in [`crates/anthill-pubsub`](crates/anthill-pubsub)):

```rust
impl Sim for MySystem {
    type Action = Action;                       // Copy + Debug
    fn enabled(&self) -> Vec<Action>;           // what could happen next (deterministic order)
    fn action_id(a: &Action) -> ActionId;       // a STABLE id, never a list index
    fn apply(&self, a: Action) -> Self;         // pure: returns the next state
    fn signature(&self) -> u64;                 // abstract state, steers exploration
    fn violated(&self) -> bool;                 // the property
}
```

Register it in your own binary and you get the whole CLI, server, UI and MCP for it:

```rust
fn main() -> std::process::ExitCode {
    anthill_cli::main_with(anthill_engine::erased::Registry::new().with("my-system", "what it does", MySystem::new))
}
```

Or test a real program without modelling it, in `anthill.toml` (Linux):

```toml
[target.checkout]
kind = "process"
command = ["/opt/app/bin/checkout", "--self-test"]   # fails by exit code, or prints ANTHILL_VIOLATION: ...
trial_timeout_secs = 120
[target.checkout.budget]
max_trials = 100
```

## For AI agents (MCP)

`anthill mcp` (stdio) or `POST /mcp` (HTTP) expose the loop an agent needs: `list_targets`, `start_run`,
`get_failure` (minimal trace plus plain-language explanation), `replay_bundle`, `cancel_run`.

```bash
claude mcp add anthill -- anthill mcp
```

Everything that comes from the system under test (step labels, states, messages) is sanitised and framed
as **untrusted data** for the agent. See [`integrations/claude-code`](integrations/claude-code).

## Dapr services

[`anthill-dapr`](crates/anthill-dapr) is a simulated Dapr sidecar: state store with ETags and first-write
concurrency, publish, and at-least-once subscription delivery, with seeded duplicates, reordering, drops,
app crashes, state conflicts and errors. Run it as a `process` target and Hermit explores the schedules.
See [`examples/dapr-orders`](examples/dapr-orders). Implemented subset only: no bindings, secrets, actors,
workflows or gRPC.

## Run it for a team

```bash
cp .env.example .env              # set real secrets
docker compose up --build -d      # PostgreSQL + server + 2 workers
```

Roles: `read` (inspect, replay), `run` (start and cancel runs, act as a worker), `admin`. Tokens are
SHA-256 hashed in memory and compared in constant time; the server refuses to listen on a non-loopback
address without tokens, rate-limits per client, caps request sizes, and defends loopback open-mode against
DNS rebinding and cross-site requests. Details and limits in [SECURITY.md](SECURITY.md). Put TLS in front of it.

## Layout

```
crates/anthill-core      Sim trait, explorer, shrinker, replay bundles, determinism doctor (std only)
crates/anthill-engine    target registry, config, reports (JUnit/SARIF/text), backend trait
crates/anthill-store     SQLite and PostgreSQL stores, leasing job queue (shared conformance tests)
crates/anthill-service   runs, jobs, results, replay: the one place all front ends call
crates/anthill-hermit    process targets under Hermit            crates/anthill-wasm   WASM backend
crates/anthill-dapr      simulated Dapr sidecar                  crates/anthill-mcp    MCP server
crates/anthill-server    REST, SSE, MCP, auth, embedded UI       crates/anthill-worker remote worker
crates/anthill-cli       the `anthill` binary                    web/                  React UI
experiments/             the early determinism experiments behind docs/determinism.md
```

## Limitations

- **Experimental.** APIs and the bundle format will change. `anthill-core` has a small, tested surface; the rest is young.
- **macOS has no system-level determinism.** Process targets need Linux x86-64 and hardware performance counters (bare metal, or a VM that exposes them). Apple Silicon cannot run Hermit.
- **Hermit** is a third-party project we do not control. We build against the maintained fork; review it before running it on anything important. Runs are roughly an order of magnitude slower than native.
- **Hermit's guest sees host files and environment as inputs.** The backend pins the environment and freezes Python's bytecode caches; other mutable host state (caches, config) can still change results. `anthill doctor` detects this.
- The doctor is probabilistic for in-process leaks: check several seeds.
- Remote workers and the database speak plain TCP/HTTP: use a private network or a TLS proxy.
- Large blobs live in the database; there is no S3 backend yet.

## Contributing

Tests first, small focused commits, `make lint test`. See [CONTRIBUTING.md](CONTRIBUTING.md). Report security
issues privately (see [SECURITY.md](SECURITY.md)).

## Acknowledgements

Inspired by FoundationDB's simulation testing, TigerBeetle's VOPR, Antithesis, and the record/replay work of
`rr` and Hermit. Anthill is an independent open-source project and is not affiliated with any of them.

## License

Apache-2.0. See [LICENSE](LICENSE).
