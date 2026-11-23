# The dhyve backend

## Experimental, not covered by the support matrix, no results published

This backend is off by default. A normal build and every release binary leave it out, and nothing in a default build
mentions it. It makes no statement about any language or program, and `docs/support-matrix.md` does not cover it.

What exists today is a machine check and nothing else. It cannot run a target, and it reports no capability.

### Building it

```bash
cargo build -p hairspring-cli --features dhyve
```

### Checking a machine

```bash
hairspring doctor --backend dhyve
```

This reads a few facts about the machine and prints one line for each prerequisite, then says what is missing and how to
fix it. It reads files and asks `kldstat` and `sysctl` questions with a five second limit each. It never loads or
unloads a kernel module, never starts a virtual machine and never changes a setting.

- On a machine that is not FreeBSD on x86-64 it runs no command, prints that the machine is not supported, and exits 4.
- On FreeBSD on x86-64 it checks the CPU's virtualisation features, that a `vmm` kernel module is loaded, that the two
  programs named by `HAIRSPRING_DHYVE_BHYVE` and `HAIRSPRING_DHYVE_BHYVECTL` exist (with their SHA-256), that no
  virtual machine from an earlier run is left, and the `machdep.panic_on_nmi` setting.
- Exit 0 means the prerequisites it can read are met. It does not show that the backend works.

The FreeBSD checks have not been run on a FreeBSD machine as part of this change.

A build without the feature answers `unknown backend` for this name, as for any other name it does not have.
