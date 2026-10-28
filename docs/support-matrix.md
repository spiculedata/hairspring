# Support matrix

This page says which languages have passed our acceptance test under Hermit, and on what. It states only what was
measured.

## Results

| Language | Status | Notes |
|---|---|---|
| C | Supported: passed our acceptance test on one configuration (Ubuntu 24.04, one Xeon host, pinned Hermit build); not tested elsewhere. | |
| C++ (clang++ and g++) | Supported: passed our acceptance test on one configuration (Ubuntu 24.04, one Xeon host, pinned Hermit build); not tested elsewhere. | Passed separately with clang++ and with g++. |
| Go | Supported: passed our acceptance test on one configuration (Ubuntu 24.04, one Xeon host, pinned Hermit build); not tested elsewhere. | Passed the acceptance test, with some trials unrated: 3 of the 400 trials (1 in the failing arm, 2 in the passing arm) gave no verdict and are not counted. |
| Rust | Supported: passed our acceptance test on one configuration (Ubuntu 24.04, one Xeon host, pinned Hermit build); not tested elsewhere. | The probe program passed, and the acceptance test passed. |
| Python | Not supported on this build: the probe program did not complete under Hermit. | |
| Java | SDK and fixture available. Not supported on this build: the probe program did not complete under Hermit. | |
| .NET | SDK and fixture available. Not supported on this build: the probe program did not complete under Hermit. | |

The SDKs and fixtures for each language are listed in the [quickstart](quickstart.md#what-is-supported).

## The one configuration

Every result above was measured on this configuration and no other:

- Ubuntu 24.04.5 LTS, kernel 6.17.0-42-generic.
- One bare-metal host with an Intel Xeon E5-1410 v2 (Ivy Bridge) CPU.
- Hermit executable with SHA-256 `666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a`, run as root.
- `hairspring 0.1.0` (engine commit `1bae6f7`), chaos mode with `max_timeslice_ns = 100000` and `skid_margin = 25`.
- Measured between 2026-10-21 and 2026-10-25.

The results apply only to that configuration. We make no claim about other hosts, kernels or Hermit builds.
