# Supply chain notes

- `anthill-core` has **no dependencies** on purpose: it is the part you must be able to trust and read in an hour.
- Rust dependencies are mainstream crates (`serde`, `toml`, `rusqlite` with bundled SQLite, `sha2`, `clap`, `axum`,
  `tokio`, `ureq`, `rust-embed`, `wasmtime`, optional `postgres`/`r2d2`). `Cargo.lock` is committed.
- The web UI depends on React, Vite, TypeScript and Vitest only; `package-lock.json` is committed.
- CI runs `cargo audit` and `npm audit --omit=dev`.
- During development a commercial dependency-intelligence service (Sonatype Guide) was configured but its access
  token had expired, so **it was not used**. Choices were made on maturity and `cargo audit`/`npm audit` results.
  If your policy requires an SCA scan, run it on `Cargo.lock` and `web/package-lock.json` before deploying.
- The Hermit backend runs a third-party tool; see [SECURITY.md](../SECURITY.md) and [determinism.md](determinism.md).
- The Docker image builds from `rust:1-bookworm` and `node:22-bookworm-slim` and runs as a non-root user; pin
  image digests in your own deployment.
