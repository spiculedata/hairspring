# Hairspring guest SDK for Java

In-source property assertions for Java programs under test, reported in
[guest event protocol v1](../../docs/guest-protocol.md). Pure Java 21, JDK classes only (no agent, no JNI, no
dependencies), Linux and macOS. The full account is in [docs/java-sdk.md](../../docs/java-sdk.md).

```java
import static hairspring.Hairspring.*;

declare(Kind.REACHABLE, "bank.recovery.ran", "recovery ran");   // a site that might never run
always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance);
sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
unreachable("bank.bad_state", "never expected");
work(1);
```

The program reports once, from a JVM shutdown hook (or by `report()`), to standard error or to `HAIRSPRING_EVENTS_FILE`.
A site that never runs is invisible unless it is `declare`d.

## Build and test

```sh
make jar        # build/hairspring-sdk.jar (module hairspring), reproducible
make test       # self-test, then the sample programs as real processes
make samples    # compile samples/hssample
```

Needs a JDK 21 (`JAVA_HOME` selects it). `cargo test -p hairspring-props --test java_sdk` runs the conformance tests
against the reference parser, including a byte-for-byte comparison with the C SDK (`HS_REQUIRE_JAVA=1` makes a missing JDK
an error).

## Layout

```
src/module-info.java        module hairspring (exports hairspring)
src/hairspring/             the SDK; Hairspring is the only public API besides Kind
test/hairspring/            SelfTest and the unit tests (no framework)
test/test-process.sh        the sample programs as real JVMs
samples/hssample/           sample programs, one per scenario
```

## Limits

* Hello and the catalog are written with the report, so a process killed before it reports leaves no lines.
* A child process must be started through `Hairspring.prepare(builder)` to get its own tag.
* Virtual threads each claim a thread slot; use platform threads for code that asserts.
* No signal-time report, fork handler or JVM agent. For Hermit results see the [support matrix](../../docs/support-matrix.md) and "Replay and paths" below.

## Replay and paths

Java is supported under Hermit on one Java-specific configuration only; see the [support matrix](../../docs/support-matrix.md)
for the exact wording and the configuration it was measured on. In short:

* Run under Hermit with `skid_margin = 35` (the other languages use 25) and the JVM flags
  `-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:auto` (the `a-prime` profile in
  `examples/language-fixtures/java/profiles-extra.tsv`), on Temurin 21.0.12.1. The fixture was reduced to 50 iterations per
  worker (`--k=50`) instead of 1,000.
* Keep the JDK install, the program jar and the working directory at the same absolute paths for recording and for replay.
  A bundle does not record the command line or the working directory ([determinism](../../docs/determinism.md)), and a JVM
  reads those paths: renaming the working directory alone changed the replay result. Re-record if the paths must change.
* Trials are slow, about 100 to 210 s each as measured, so this suits nightly runs rather than every commit.
