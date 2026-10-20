// Package hairspring is the Go SDK for Hairspring's guest event protocol v1 (docs/guest-protocol.md): in-source
// property assertions that an unmodified Linux program reports to the Hairspring engine.
//
// # Use
//
// Declare each assertion once, before the workload starts, and run the workload under [Main]:
//
//	var counterExact = hairspring.Always("app.counter_exact", "no update may be lost")
//
//	func main() { os.Exit(hairspring.Main(run)) }
//
//	func run() int {
//		counterExact.Check(total == want, hairspring.Vals{total, want})
//		return 0
//	}
//
// A site is evaluated with [Handle.Check] (an always, always_or_unreachable or sometimes site) or [Handle.Hit] (a
// reachable or unreachable site). Evaluating a site is a few atomic operations: no system call, no lock and no
// allocation, so a call site can sit inside the code window that a race needs without closing it. The report is
// written once, when [Main] (or [Run]) finishes: a hello line and the catalog before the workload starts, then one
// assert line per site, a work line and an end line.
//
// # Where the report goes
//
// [Main] writes to standard error, one write call per line, so concurrent writers to a pipe cannot interleave inside
// a line. If HAIRSPRING_EVENTS_FILE names a file, the lines go there instead (opened for append, a symbolic link
// is refused, and the variable is ignored in a set-uid or set-gid process). If that file cannot be opened the report
// falls back to standard error and the end line counts the failure in "dropped". HAIRSPRING_EVENTS=off writes
// nothing. HAIRSPRING_PROC sets the process tag (default "p1"); a backend sets it for child processes.
//
// # Limits
//
//   - Goroutines are not threads: the SDK sends no thread lines (end.threads is 0), every witness carries the
//     thread key "go", and its ordinal is the site-wide evaluation number rather than a per-thread one.
//   - The report is written when the function given to [Main] returns. os.Exit, a panic and a fatal signal bypass
//     it, and the hello line says signal_handlers=false. Return the exit status from the function instead of
//     calling os.Exit inside it.
//   - Sites must be declared before [Main] starts the workload; a declaration after that is an error.
//   - Building with -tags hs_violation_only makes [Main] print only HAIRSPRING_VIOLATION: lines (the legacy
//     marker), which is how a build can be checked for neutrality against the full report.
//
// The package has no dependencies outside the standard library.
package hairspring
