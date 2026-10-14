# Anthill Buildout Plan

Companion to `SPEC.md` (what and why, plus measurements). This is the how: interfaces, backend, frontend, frameworks, MCP, phasing. Status: draft for review. Items marked **(verify)** rest on my recollection of the ecosystem and must be checked before we commit.

## 0. Product shape in one paragraph

Anthill explores many executions of a system under test, finds a violated property, shrinks it, and gives you a replayable repro. The system under test runs on one of several **backends** that differ in how much determinism they give. Everything above the backend (store, explorer scheduling, server, UI, MCP, CI integrations) is shared.

```
            ┌──────────── clients ────────────┐
   CLI  ·  Web UI  ·  MCP (agents)  ·  CI Action  ·  Rust/other SDKs
            └──────────────┬───────────────────┘
                     anthill-server  (REST + SSE, auth, scheduler)
                 ┌───────────┼─────────────┐
              store        job queue     blob store
         (SQLite/Postgres) (PG SKIP LOCKED) (fs/S3)
                 └───────────┬─────────────┘
                        workers (pull jobs)
          ┌──────────────┬───┴───────────┬───────────────┐
      SimBackend     WasmBackend    HermitBackend    (future VmBackend)
      in-process     Wasmtime       Linux x86-64     microVM / hypervisor
      mac + linux    mac + linux    experimental     out of scope for now
```

## 1. Backends (the key abstraction)

All backends implement one trait: given a target, a seed/choice plan and a fault plan, produce an `Outcome { trace_hash, events, verdict, artifacts }`.

| Backend | Exploration style | Time travel | State inspection | Platforms | Status |
|---|---|---|---|---|---|
| Sim (Rust `Sim` trait) | **Tree**: snapshot, branch, shrink by action ID | Full: restore any node | Full (typed state) | macOS, Linux | Core done (M1) |
| Wasm (Wasmtime) | Tree if we snapshot linear memory; else seed-space | Good in principle | Memory view | macOS, Linux | Unproven; spike first |
| Hermit | **Seed-space**: `--sched-seed`, `--sched-heuristic=random`, `--chaos`; black-box | Replay-to-step-N only (re-run with same seed, scrub the log); no state restore | Logs, syscalls, exit status | x86-64 Linux, PMU preferred | Measured working, experimental |

Consequence for the data model: two exploration modes share a store. `Node` graph (tree mode) and `Trial { params, outcome }` list (seed mode). The UI must not promise "restore any state" on Hermit runs.

## 2. Workspace layout (Rust, Cargo workspace)

```
crates/
  anthill-core      Sim trait, explore, shrink, replay, bundle, doctor   (done)
  anthill-sim       simulated runtime: virtual clock, net, disk, fault plans
  anthill-store     schema + migrations; SQLite and Postgres impls; blob store trait
  anthill-backend   Backend trait + Sim/Wasm/Hermit adapters
  anthill-server    axum REST + SSE, scheduler, auth, embeds web UI
  anthill-worker    pulls jobs, runs backends, streams events
  anthill-cli       `anthill` binary (local mode embeds server+worker)
  anthill-mcp       MCP server (stdio local, streamable HTTP remote)
  anthill-sdk       user-facing macros/assertions (always, sometimes, reachable)
web/                React SPA
integrations/       github-action/, claude-code plugin/skill, dapr-sim/
```

Rule: `anthill-core` stays dependency-free (std only). Each new dependency goes through a supply-chain check before it is added.

## 3. Interfaces

### 3.1 CLI
`anthill init | test | run | replay <bundle> | shrink <bundle> | doctor | serve | worker | bundle export/import | results`
- Local mode: single process, SQLite, no network. This is the default dev experience.
- `--remote <url>` submits to a server; the same command works either way.
- Output formats: human, JSON, JUnit, SARIF (for CI annotations).

### 3.2 Target definition: `anthill.toml`
```toml
[target.pubsub]
kind = "sim"            # sim | wasm | process
# kind = "process": run under Hermit on Linux
command = ["./bin/consumer"]
budget  = { steps = 1_000_000, wall = "5m" }
faults  = ["partition", "clock-skew", "crash"]
properties = ["exactly_once", "no_lost_messages"]
```

### 3.3 SDK and properties
- Rust first: `Sim` trait + attribute macros; assertions `always!`, `sometimes!`, `reachable!`, with a message and structured context.
- `sometimes!` matters: it proves the explorer reached interesting states, not only that nothing broke.
- Other languages: run as `process` target (Hermit) or compile to WASM; a thin assertion protocol (JSON lines on a fixed fd or env-configured socket) so any language can report assertions. Hegel interop: **(verify)** read its protocol before designing the adapter.

### 3.4 Server API
REST (OpenAPI generated) plus SSE for live progress. Workers lease jobs over the same API (long-poll), so one auth model covers everything. gRPC is deliberately skipped until a proven need.
Core resources: `targets`, `runs`, `trials`/`nodes`, `failures`, `bundles`, `workers`.

### 3.5 MCP server (agent loop is a differentiator)
Goal: let Claude Code (or any agent) run the loop *write property → explore → read minimal repro → fix → re-run* without a human relaying logs.
Tools: `list_targets`, `start_run`, `get_run`, `list_failures`, `get_failure` (minimal trace plus structured explanation), `replay_bundle`, `shrink_bundle`, `diff_nodes`, `cancel_run`.
Resources: `anthill://runs/{id}`, `anthill://failures/{id}`, `anthill://bundles/{id}`.
Transport: stdio for local, streamable HTTP with bearer token for server. Read-only token scope for CI bots. Use the official Rust MCP SDK **(verify its maturity)**, else a thin hand-rolled JSON-RPC layer.
Safety: agent-supplied target commands execute code, so MCP `start_run` only runs targets already declared in `anthill.toml` or approved on the server. No arbitrary command strings.
Also ship a Claude Code skill/plugin (`/anthill`) that wraps CLI and MCP.

## 4. Backend services

- **Language/framework:** Rust, tokio, axum. Reason: shared types with the engine, one static binary, easy self-hosting.
- **Store:** SQLite (local) and Postgres (server) behind one trait; migrations with `sqlx` or `refinery` **(verify)**. Blobs (snapshots, recordings, logs): filesystem or S3-compatible via the `object_store` crate **(verify)**, content-addressed and chunked.
- **Scheduling:** Postgres `FOR UPDATE SKIP LOCKED` job queue with leases and heartbeats. No Redis/Kafka for MVP.
- **Auth:** static API tokens first; OIDC later. Roles: admin, run, read-only.
- **Observability:** `tracing` plus OTLP export; Prometheus metrics endpoint.
- **Packaging:** static binary, Docker image, `docker-compose.yml` (server + Postgres + MinIO + one worker). Hermit workers need `ptrace`, user/PID/mount namespaces and PMU access; document `--cap-add SYS_PTRACE`, relaxed seccomp, and `perf_event_paranoid<=1`, or prefer bare-metal/VM workers. Test exactly what the container profile permits before promising it.
- **Hermit worker specifics:** run each trial inside the Hermit sandbox with a fixed `--epoch`; keep the program and data outside `/tmp` and home (Hermit replaces `/tmp`, user namespaces cannot traverse mode-750 homes); external network is blocked by design, so dependencies must be mocked or run on loopback.

## 5. Frontend

- **Stack:** React + TypeScript + Vite, TanStack Query for data, Tailwind with a headless component kit; served from the Rust binary (embedded assets) so there is nothing extra to deploy. Alternative considered: Leptos (all-Rust). Rejected for contributor reach and ecosystem; revisit if the team prefers.
- **Large graphs:** canvas/WebGL for the node tree (thousands to millions of nodes), not SVG. Candidates: sigma.js or a custom canvas layer **(verify)**.
- **Screens:**
  1. Runs list and live run (progress, coverage curve, property status: always / sometimes hit counts).
  2. Failure view: minimal trace as a timeline, grouped by actor; plain-language "what happened"; link to bundle.
  3. Replay player: step forward/back. Tree backends restore from the nearest snapshot. Hermit shows the recorded schedule and logs and re-runs by seed (say so in the UI).
  4. Compare: diff two nodes or a failing and a passing neighbour.
  5. Targets/config, workers/health, tokens.
- **Design:** accessibility and keyboard navigation from day one; dark mode.

## 6. Integrations
- GitHub Action: run on PR, upload bundle artifacts, SARIF annotations, comment with the minimal repro.
- `cargo anthill` wrapper for Rust projects.
- **Dapr simulation:** a simulated sidecar implementing pub/sub, state store, bindings, and secrets APIs with configurable faults (redelivery, timeout, partial failure). Apps under Hermit or WASM talk to the simulated sidecar on loopback. A natural first domain for Dapr-based microservices; do after the core path works.

## 7. Phasing

| Phase | Deliver | Exit criterion |
|---|---|---|
| M1 (done) | `anthill-core` | 21 tests green, doctor catches HashMap leak |
| M2 | `anthill-store` + `anthill-cli` local mode, bundle export, SQLite | `anthill test` finds, shrinks, stores and replays the pub/sub bug from a fresh clone |
| M3 | `anthill-server` + worker + **MCP** (read tools first) | Claude Code can list runs, fetch a failure and replay it via MCP |
| M4 | Web UI: runs, failure view, replay player | A non-author can find and understand a failure from the UI alone |
| M5 | `HermitBackend` (Linux, experimental) | 3 real programs, 100% repeatable by seed, honest overhead numbers; container profile documented |
| M6 | WASM spike then backend | Same program replays identically on macOS and Linux; snapshot/fork works |
| M7 | Dapr simulator; GitHub Action | A real Dapr service tested end to end in CI |
| M8 | Postgres/S3 multi-worker, auth, packaging | `docker compose up` gives a working cluster |

Why MCP early (M3, not last): it is cheap once the server API exists, and it makes every later feature usable by an agent immediately.

## 8. Risks and honest unknowns
1. Hermit: small third-party fork, unaudited, x86-64 Linux only, large slowdown (rough estimate ~19x, to be re-measured), first-run determinism wrinkle under investigation. Mitigation: isolate behind the Backend trait; label experimental.
2. Tier 1 requires users to model systems as state machines or run on a deterministic runtime. Real-code porting cost is unmeasured beyond a toy plus one modelled service.
3. UI scope creep: gate behind M2-M3 working end to end first.
4. Supply chain: every dependency vetted; prefer std/tiny crates in core.
5. Licence and naming undecided.

## 9. Decisions needed
1. Name and licence (Apache-2.0 proposed).
2. Is MCP in M3 acceptable, or first-class from M2?
3. React vs Leptos for the UI.
4. Rust-only SDK for the MVP, or Go in parallel?
5. Do we build the Dapr simulator before or after Hermit backend?
6. Open-source from day one (public repo) or private until M4?
