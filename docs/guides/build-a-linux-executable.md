# Build a Linux executable for a process target

A `process` target runs an existing Linux x86-64 program under Hermit. The `command` in `hairspring.toml` must point at
something that machine can execute: a native executable, or the interpreter or runtime for your language plus its
arguments. This page shows how to produce that for each language, from a Mac and on Linux, and which of the steps were
run.

Before you build, check that the machine that will run it can use the process backend at all:
[is my machine suitable?](is-my-machine-suitable.md). The support status of each language is the
[support matrix](../support-matrix.md); this page does not change it, and it does not say a language works beyond its
row.

## Rules of thumb

- **Build for Linux on x86-64.** An executable built on a Mac for macOS will not run. Cross-compile where the language
  supports it (Go does), otherwise build in a `linux/amd64` container or on Linux.
- **Use the distribution you will run on.** The one measured distribution is Ubuntu 24.04 (the matrix: Ubuntu 24.04.5
  LTS). A dynamically linked program uses the machine's own C library at run time; Ubuntu 24.04 has glibc 2.39 (it is
  what `ldd --version` printed in the `ubuntu:24.04` container used for this page). Hermit's replay depends on the
  machine's CPU, kernel and loader, and a recording is tied to the loader and libc hashes (see
  [determinism](../determinism.md)), so build and replay on the same distribution release where you can.
- **Use an absolute path in `command`**, and keep it the same for recording and replay. See the Java note below for why
  this matters most for a JVM.
- **Static or dynamic.** The repository's own fixtures are built as follows: Go static, Rust static against musl, C and
  C++ with the SDK archive linked into an ordinary dynamically linked executable (see
  [`examples/language-fixtures/c/Makefile`](../../examples/language-fixtures/c/Makefile)). The C SDK must be linked
  into the executable itself, not into a shared object ([c-sdk.md](../c-sdk.md)). A fully static glibc build is
  shown below because it avoids depending on the host's libc; it was built and checked with `file` here, but it was
  not run under Hermit here, and the matrix result for C and C++ is for the fixtures' build, not for a static glibc
  build.
- Every build line below that says "run here" was run on an Apple silicon Mac, or in a `linux/amd64` Ubuntu 24.04
  container on Docker Desktop on that Mac. Nothing was run under Hermit, which needs Linux x86-64 with a PMU.

## Go

Go cross-compiles, so this works from a Mac and needs no container. Run here with Go 1.27.1:

```bash
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o driver-linux ./driver
file driver-linux
```

```text
driver-linux: ELF 64-bit LSB executable, x86-64, version 1 (SYSV), statically linked, Go BuildID=..., with debug_info, not stripped
```

`CGO_ENABLED=0` keeps the result free of the C library. The Go example in the wiki and the repository's Go fixture use
the longer form, which also makes the build reproducible:

```bash
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 \
  go build -trimpath -buildvcs=false -ldflags=-buildid= -o seats-linux .
```

That form was run here too (`file` reports `statically linked`). On Linux, leave out `GOOS` and `GOARCH`. Without
`CGO_ENABLED=0`, a Linux build that imports `net` or `os/user` can link the C library dynamically; that case was not
run here.

The SDK needs Go 1.24 or newer. See [Guest SDKs by language](https://github.com/spiculedata/hairspring/wiki/Guest-SDKs-by-Language#go).

## Rust

On Linux, a plain `cargo build --release` gives a dynamically linked glibc executable. A static one comes from the
musl target. Run here, in an Ubuntu 24.04 `linux/amd64` container with Rust 1.99.1 installed by rustup, building a
program that uses the Rust SDK through a path dependency:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/demo
```

```text
.../demo: ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, BuildID[sha1]=..., not stripped
```

The native build in the same container gave `dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2`. The musl
build ran in the container and printed its `assert` line. Without the target installed the build stops with
``error[E0463]: can't find crate for `std` `` and the hint `rustup target add x86_64-unknown-linux-musl`.

The static musl build is the one the project's fixture uses (`x86_64-unknown-linux-musl`; its build record adds only
`--remap-path-prefix`). With musl the SDK's `end.status` is advisory ([rust-sdk.md](../rust-sdk.md)); the engine always
uses the operating system's wait status.

**From a Mac.** Homebrew's Rust has no rustup and no musl standard library, so the build cannot find `std` for the
target (the same `error[E0463]` you get in the container above when the target is missing). With rustup you can add the target, but linking a Linux executable also needs a linker that can produce
one, and that was not tried here. The container route was run as one command, with a `Cargo.toml` whose dependency is
`hairspring-sdk = { path = "/hairspring/sdks/rust" }` and the repository mounted at `/hairspring`:

```bash
docker run --rm --platform linux/amd64 -v /path/to/hairspring:/hairspring:ro -v "$PWD":/src -w /src ubuntu:24.04 sh -c '
  apt-get update -qq && apt-get install -y -qq build-essential curl ca-certificates &&
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal &&
  . /root/.cargo/env && rustup target add x86_64-unknown-linux-musl &&
  cargo build --release --target x86_64-unknown-linux-musl'
file target/x86_64-unknown-linux-musl/release/demo
```

It exited 0 and `file` on the Mac reported `ELF 64-bit LSB pie executable, x86-64, ... static-pie linked`. The command
downloads the toolchain on every run; mount a volume over `/root/.cargo` and `/root/.rustup` if you build often.

## C and C++

Build the SDK archive, then link it into your program. On Linux, with `build-essential` installed. Run here in an Ubuntu
24.04 `linux/amd64` container (gcc 13.3), with the demo programs from the wiki's SDK page:

```bash
make -C sdks/c lib CC=cc BUILD=/tmp/csdk                      # writes /tmp/csdk/libhairspring.a
cc  -O1 -pthread -Isdks/c/include demo.c  /tmp/csdk/libhairspring.a -o demo-dynamic
cc  -O1 -pthread -static -Isdks/c/include demo.c  /tmp/csdk/libhairspring.a -o demo-static
g++ -std=c++17 -O1 -pthread -static -Isdks/c/include demo.cc /tmp/csdk/libhairspring.a -o demo-cc-static
file demo-dynamic demo-static demo-cc-static
```

```text
demo-dynamic:    ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2, ...
demo-static:     ELF 64-bit LSB executable, x86-64, version 1 (GNU/Linux), statically linked, ...
demo-cc-static:  ELF 64-bit LSB executable, x86-64, version 1 (GNU/Linux), statically linked, ...
```

Both static programs ran and printed their `assert` line for `app.counter_exact`. `BUILD=` keeps the library out of
the checkout; without it the archive goes to `sdks/c/build/<compiler><optimisation>/`.

**From a Mac**, build in a container with the repository mounted read-only and your source directory mounted for the
output. Run here with `demo.c` and `demo.cc` in the current directory:

```bash
docker run --rm --platform linux/amd64 -v /path/to/hairspring:/hairspring:ro -v "$PWD":/src -w /src ubuntu:24.04 sh -c '
  apt-get update -qq && apt-get install -y -qq build-essential &&
  make -C /hairspring/sdks/c lib CC=cc BUILD=/tmp/csdk &&
  cc -O1 -pthread -static -I/hairspring/sdks/c/include demo.c /tmp/csdk/libhairspring.a -o demo &&
  g++ -std=c++17 -O1 -pthread -static -I/hairspring/sdks/c/include demo.cc /tmp/csdk/libhairspring.a -o demo-cc'
file demo demo-cc
```

On the Mac, `file` reported `ELF 64-bit LSB executable, x86-64, ... statically linked` for both.

The C++ notes (clang and g++ differences, templates, which compilers were tried) are in
[c-sdk.md](../c-sdk.md#c-templates-and-the-catalog-section).

## Java

A Java target runs the JVM, so the executable you point at is `java` and the program is a jar or class path. The matrix
status for Java is specific:

- the configuration that passed used `skid_margin = 35`, the JVM flags
  `-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:auto`, Temurin 21.0.12.1 and a reduced
  fixture; see the [support matrix](../support-matrix.md) for the exact wording and its limits;
- a replay matched only from the same absolute paths as the recording: the JDK install, the program jar and the working
  directory. Keep them fixed.

The SDK builds with `make -C sdks/java jar` (it writes `sdks/java/build/hairspring-sdk.jar`) and is pure Java 21. The
settings the matrix records for the configuration that passed are `chaos = true`, `max_timeslice_ns = 100000`,
`skid_margin = 35` (Java's value; the other languages used 25) and the JVM flags above. Put together, the shape of a
target (not run here, because it needs Hermit; the paths are placeholders):

```toml
[target.my-java]
kind = "process"
command = [
  "/opt/jdk-21/bin/java", "-Xint", "-XX:+UseSerialGC", "-XX:ActiveProcessorCount=1", "-XX:-UsePerfData", "-Xshare:auto",
  "-cp", "/opt/app/app.jar:/opt/app/hairspring-sdk.jar", "com.example.Main",
]
chaos = true
max_timeslice_ns = 100000
skid_margin = 35
# Also set trial_timeout_secs, and wall_secs in [target.my-java.budget]; see below.
```

The matrix does not record the trial timeout or the wall-clock budget that were used, so no value is given here. You
must set both, and well above the trial time: the matrix says a Java trial takes about 105 s alone and about 210 s as
run (up to three at once), while the defaults are 60 s for one trial and 60 s for the whole run, which a Java trial
would exceed. A trial that times out is inconclusive, never a pass.

See the matrix for the configuration that was measured, and [java-sdk.md](../java-sdk.md).

## Python

Python is **not supported on this build**: the matrix says the probe program did not complete under Hermit. The SDK works
natively and its tests pass ([python-sdk.md](../python-sdk.md)), but do not expect a Python program to run as a `process`
target. If you want to try on your own machine, the `command` would be the interpreter's absolute path and your script;
that was not tried here.

## .NET

.NET is **not supported on this build**: the SDK and fixture are available, and the probe program did not complete under
Hermit. The SDK is for `net10.0` and needs a .NET 10 SDK ([dotnet-sdk.md](../dotnet-sdk.md)); a program built with it
runs as `dotnet YourProgram.dll`, so a target would need the `dotnet` runtime installed on the Linux machine. That was not
tried here.

## What was and was not run

| Step | Run? |
|---|---|
| Go cross-build to linux/amd64 from the Mac, `file` check | Yes |
| Rust musl build and native build in an Ubuntu 24.04 container, `file` check, run in the container; the musl build also as one `docker run` from the Mac | Yes |
| Rust musl cross-build natively on the Mac | No: Homebrew's Rust has no rustup. Only the container route was run |
| C and C++ static builds in an Ubuntu 24.04 container, from the Mac through `docker run` and in a container shell, `file` check, run in the container | Yes |
| Any build for Java, Python or .NET | No |
| Anything under Hermit | No |
