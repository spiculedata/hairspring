# Supply chain notes

- `hairspring-core` has **no dependencies** on purpose: it is the part you must be able to trust and read in an hour.
- Rust dependencies are mainstream crates (`serde`, `toml`, `rusqlite` with bundled SQLite, `sha2`, `clap`, `axum`,
  `tokio`, `ureq`, `rust-embed`, `wasmtime`, optional `postgres`/`r2d2`). `Cargo.lock` is committed.
- The web UI depends on React, Vite, TypeScript and Vitest only; `package-lock.json` is committed.
- CI runs `cargo audit` and `npm audit --omit=dev`.
- During development a commercial dependency-intelligence service (Sonatype Guide) was configured but its access
  token had expired, so **it was not used**. Choices were made on maturity and `cargo audit`/`npm audit` results.
  If your policy requires an SCA scan, run it on `Cargo.lock` and `web/package-lock.json` before deploying.
- The Hermit backend runs a third-party tool; see [SECURITY.md](../SECURITY.md) and [determinism.md](determinism.md).
- The installers ([quickstart](quickstart.md)) pin what they can: `scripts/install-hermit.sh` builds the
  [maintained fork](https://github.com/spiculedata/hermit) at one commit, verifies the replay patch by sha256 before
  cloning, and builds with `cargo build --release --locked` on a dated nightly toolchain. A rebuilt Hermit is not
  expected to have the same sha256 as ours, so the script prints and records the hash of what it built rather than
  comparing it with a published one. `scripts/install.sh --version <tag>` checks a downloaded binary against the
  release's `SHA256SUMS`, which comes from the same place as the binary: it catches a corrupt download, not a
  compromised release. Pass `--sha256` with a hash you obtained another way to pin it. The release workflow builds with
  `--locked` and publishes nothing to a package registry; it creates a draft release for a maintainer to review.
- The Docker image builds from `rust:1-bookworm` and `node:22-bookworm-slim` and runs as a non-root user; pin
  image digests in your own deployment.
