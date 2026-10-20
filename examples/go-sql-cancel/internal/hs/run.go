package hs

import "os"

// Config says where and what Run writes. It is a value; Run never modifies it.
type Config struct {
	Proc          string             // process tag; "p1" for a root process
	Write         func([]byte) error // called once per line, with the whole line
	Disabled      bool               // HAIRSPRING_EVENTS=off: write nothing
	ViolationOnly bool               // write only HAIRSPRING_VIOLATION: lines, for the neutrality gate
}

const defaultProc = "p1"

// configFromEnv reads HAIRSPRING_PROC and HAIRSPRING_EVENTS=off. An invalid tag falls back to the root tag.
// The events file setting (HAIRSPRING_EVENTS_FILE) is not supported: the shim writes to the sink only.
func configFromEnv(getenv func(string) string, write func([]byte) error, violationOnly bool) Config {
	proc := getenv("HAIRSPRING_PROC")
	if len(proc) > 64 || !identPattern.MatchString(proc) {
		proc = defaultProc
	}
	return Config{Proc: proc, Write: write, Disabled: getenv("HAIRSPRING_EVENTS") == "off", ViolationOnly: violationOnly}
}

// Run sends hello and the catalog, freezes the catalog, runs fn, then sends the asserts, work and end lines (or,
// in ViolationOnly mode, only the violation lines). It returns fn's result, which is what the caller should exit
// with. A panic in fn propagates and no report is sent.
func Run(c *Catalog, cfg Config, fn func() int) int {
	c.Freeze()
	start := c.Snapshot()
	var dropped uint64
	send := func(l []byte) {
		if !cfg.Disabled && cfg.Write(l) != nil {
			dropped++
		}
	}
	if !cfg.ViolationOnly {
		send(helloLine(cfg.Proc, catalogHash(start)))
		for _, s := range start.Sites {
			send(catalogLine(cfg.Proc, s.Site))
		}
	}
	status := fn()
	end := c.Snapshot()
	if cfg.ViolationOnly {
		for _, v := range end.Violations() {
			send(violationLine(v))
		}
		return status
	}
	for _, s := range end.Sites {
		send(assertLine(cfg.Proc, s))
	}
	send(workLine(cfg.Proc, end.Ops))
	send(endLine(cfg.Proc, int64(status), uint64(len(end.Sites)), end.Evals(), dropped))
	return status
}

// std is the process-wide catalog behind Declare, AddWork and Main.
var std = NewCatalog()

// Declare adds a site to the process-wide catalog; call it from a package-level var or init. An invalid or
// conflicting site is a programming error and panics at startup.
func Declare(s Site) *Handle {
	h, err := std.Declare(s)
	if err != nil {
		panic(err)
	}
	return h
}

// AddWork adds to the operations count of the process-wide catalog.
func AddWork(n uint64) { std.AddWork(n) }

// Main runs fn under the process-wide catalog, writing lines to standard error (one write call per line, so
// concurrent writers to a pipe cannot interleave inside a line). Use it as os.Exit(hs.Main(run)).
func Main(fn func() int) int {
	write := func(l []byte) error { _, err := os.Stderr.Write(l); return err }
	return Run(std, configFromEnv(os.Getenv, write, violationOnlyBuild), fn)
}
