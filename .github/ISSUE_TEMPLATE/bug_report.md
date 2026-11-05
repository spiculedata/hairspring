---
name: Bug report
about: Something does not work as documented
labels: bug
---

Do not report security issues here; see SECURITY.md.

## What happened

Expected:

Actual:

## Reproduction

Steps or command (with the seed, if there is one):

Does `hairspring replay <bundle>` reproduce it?  yes / no / no bundle

## Environment

- Output of `hairspring --version` (the whole line; a bare `hairspring 0.1.0` with no commit is the v0.1-RC1 pre-release):
- OS and kernel (`uname -a`):
- Backend: sim / wasm / process
- Hermit version or commit (process backend only):
- Installed from: source / release build / Docker

## `hairspring doctor` output

Output of `hairspring doctor` and, if you use the `process` backend, of `hairspring doctor --process-backend`:

```text
```

## Config

Relevant part of `hairspring.toml`, with secrets removed:

```toml
```
