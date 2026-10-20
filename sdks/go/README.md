# Hairspring Go SDK

In-source property assertions for Go programs, reported to Hairspring through
[guest event protocol v1](../../docs/guest-protocol.md). A pure Go module with no dependencies outside the standard
library; it needs Go 1.24 or newer.

```go
import hs "github.com/spiculedata/hairspring/sdks/go"

var counterExact = hs.Always("app.counter_exact", "no update may be lost")

func main() { os.Exit(hs.Main(run)) }

func run() int {
	// ... the workload ...
	counterExact.Check(total == want, hs.Vals{int64(total), int64(want)})
	return 0
}
```

The package documentation (`go doc github.com/spiculedata/hairspring/sdks/go`) is the reference. In short:

- **Declare** each site once, before the workload starts: `hs.Always`, `hs.AlwaysOrUnreachable`, `hs.Sometimes`,
  `hs.Reachable` and `hs.Unreachable` record the caller's file and line; `hs.Declare(hs.Site{...})` takes the fields
  explicitly. The same id at two sites is one property.
- **Evaluate** with `Handle.Check(ok, vals)` or `Handle.Hit(vals)`. That is a few atomic operations: no system call,
  lock or allocation, so a site can sit inside the code window a race needs. Up to four integers (`hs.Vals`) are kept
  for the first true and the first false evaluation.
- **Run** the workload under `hs.Main`. It sends the hello line and the catalog first, and the asserts, a work line
  and the end line when the function returns. Return the exit status from the function; `os.Exit` and a panic bypass
  the report.

## Where the report goes

Standard error, one write call per line. `HAIRSPRING_EVENTS_FILE=path` sends it to a file instead (opened for append,
symbolic links refused, ignored in a set-uid or set-gid process); if the file cannot be opened the report falls back
to standard error and `end.dropped` counts it. `HAIRSPRING_EVENTS=off` writes nothing. `HAIRSPRING_PROC` sets the
process tag.

## Limits

- Goroutines are not threads. The SDK sends no thread lines, every witness carries the thread key `go`, and its
  ordinal is the site-wide evaluation number.
- The report is written when the function given to `hs.Main` returns. `os.Exit`, a panic and a fatal signal bypass it
  (`signal_handlers` is false in the hello line).
- `-tags hs_violation_only` makes `hs.Main` print only `HAIRSPRING_VIOLATION:` lines.

## Tests

```sh
go test ./...                     # unit tests, and conformance against the shared goldens
go vet ./...
cargo test -p hairspring-props --test go_sdk   # the reference parser folds what the sample programs write
```

`conformance_test.go` checks every line the SDK writes against an independent reading of the protocol (field types,
limits, padding, cross-checks such as `end.sites` against the catalog), and checks that reading against the shared
goldens in [`../conformance`](../conformance): it accepts every line of the valid goldens and rejects every line of
`invalid.events`. The Rust test builds `internal/hssample` and folds its output with the reference parser. The sample
program is a test fixture, not part of the SDK's API.

## Relation to `examples/go-sql-cancel`

That example has its own copy of this code in `internal/hs`, from before the SDK existed. The example is left as it
was: its page and its recorded binary hashes describe that exact source, and moving it onto this module would change
the binaries. This module is the maintained one and adds the events file, the located constructors and the
conformance suite.
