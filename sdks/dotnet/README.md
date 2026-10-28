# Hairspring guest SDK for .NET

In-source property assertions for .NET programs under test, reported in
[guest event protocol v1](../../docs/guest-protocol.md). C# for `net10.0`, base class library only (no NuGet package, no
agent, no native code of its own), Linux and macOS. The full account is in [docs/dotnet-sdk.md](../../docs/dotnet-sdk.md).

> **Status.** The SDK, its tests and a fixture exist. A .NET program has **not yet been run under Hermit**, and **.NET is not
> supported on this build**.

```csharp
using static Hairspring.Hs;

Declare(Kind.Reachable, "bank.recovery.ran", "recovery ran");   // a site that might never run
Always(balance >= 0, "bank.balance_non_negative", "no overdraft", [balance]);
Sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
Unreachable("bank.bad_state", "never expected");
Work(1);
```

The program reports once, when the process exits (or by `Report()`), to standard error or to `HAIRSPRING_EVENTS_FILE`. A site
that never runs is invisible unless it is `Declare`d. Call `Hs.Start()` first thing in `Main` if a background thread might be
the first to use the SDK.

## Build and test

```sh
make dll        # build/artifacts/bin/Hairspring/release/Hairspring.dll
make test       # self-test, then the sample programs as real processes
make samples    # build samples/Sample
```

Needs the .NET 10 SDK (`DOTNET=/path/to/dotnet` selects it). Inside this repository the root [`global.json`](../../global.json)
asks for 10.0.100 or later in the same major and minor version, so any 10.0 SDK works for development. The Linux build records
and CI use exactly 10.0.402, written in [`scripts/record-global.json`](../../scripts/record-global.json). `cargo test -p hairspring-props --test dotnet_sdk` runs the
conformance tests against the reference parser, including a byte-for-byte comparison with the C SDK (`HS_REQUIRE_DOTNET=1` makes
a missing SDK an error).

## Layout

```
src/Hairspring.csproj      the SDK; Hs is the only public API besides Kind
tests/SelfTest/            the unit tests (no framework)
test/test-process.sh       the sample programs as real processes
samples/Sample/            sample programs, one per scenario
```

## Limits

* Hello and the catalog are written with the report, so a process killed before it reports leaves no lines. An unhandled
  exception is reported (as `SIGABRT`); `SIGTERM`, `SIGINT`, `Environment.FailFast` and `SIGKILL` are not.
* A child process must be started through `Hs.Prepare(startInfo)` to get its own tag.
* Every thread that calls the SDK takes a slot, thread-pool threads included; use dedicated threads for code that asserts.
* No signal-time report, fork handler or startup hook. For Hermit results see the [support matrix](../../docs/support-matrix.md).
