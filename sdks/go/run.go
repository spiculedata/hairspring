package hairspring

import (
	"io"
	"os"
)

// Config says where and what Run writes. It is a value; Run never modifies it.
type Config struct {
	Proc          string             // process tag; "p1" for a root process
	Write         func([]byte) error // called once per line, with the whole line
	Dropped       uint64             // events already lost before Run starts (reported in the end line)
	Unstable      bool               // the tag was derived from a pid, so it can differ between runs (hello.tag_stable)
	Disabled      bool               // HAIRSPRING_EVENTS=off: write nothing
	ViolationOnly bool               // write only HAIRSPRING_VIOLATION: lines, for the neutrality gate
}

// configFromEnv reads HAIRSPRING_PROC, HAIRSPRING_PROC_PID and HAIRSPRING_EVENTS=off. An invalid tag falls back to the
// root tag and is counted in Dropped, as the C SDK does; a process spawned by another instrumented process gets a
// derived tag (see identity.go).
func configFromEnv(getenv func(string) string, pid int, write func([]byte) error, violationOnly bool) Config {
	id := resolveIdentity(getenv, pid)
	cfg := Config{Proc: id.tag, Write: write, Unstable: id.unstable, Disabled: getenv("HAIRSPRING_EVENTS") == "off", ViolationOnly: violationOnly}
	if id.refused {
		cfg.Dropped = 1
	}
	return cfg
}

// Run sends hello and the catalog, freezes the catalog, runs fn, then sends the asserts, work and end lines (or,
// in ViolationOnly mode, only the violation lines). It returns fn's result, which is what the caller should exit
// with. A panic in fn propagates and no report is sent.
func Run(c *Catalog, cfg Config, fn func() int) int {
	c.Freeze()
	start := c.Snapshot()
	dropped := cfg.Dropped
	send := func(l []byte) {
		if !cfg.Disabled && cfg.Write(l) != nil {
			dropped++
		}
	}
	if !cfg.ViolationOnly {
		send(helloLine(cfg.Proc, catalogHash(start), cfg.Unstable))
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
	send(endLine(cfg.Proc, int64(status), uint64(min(len(end.Sites), maxSitesPerProcess)), end.Evals(), dropped))
	return status
}

// std is the process-wide catalog behind Declare, Always, AddWork and Main.
var std = NewCatalog()

// Declare adds a site to the process-wide catalog; call it from a package-level var or init. An invalid or
// conflicting site is a programming error and panics at startup.
func Declare(s Site) *Handle { return declareOrPanic(std, s) }

func declareOrPanic(c *Catalog, s Site) *Handle {
	h, err := c.Declare(s)
	if err != nil {
		panic(err)
	}
	return h
}

// AddWork adds to the operations count of the process-wide catalog.
func AddWork(n uint64) { std.AddWork(n) }

// processEnv is everything Main takes from the operating system, so tests can supply it.
type processEnv struct {
	getenv        func(string) string
	stderr        io.Writer
	privileged    bool // started with elevated rights: the events file setting is not trusted
	violationOnly bool
	pid           int
	setenv        func(key, value string) // exports the tag for child processes
}

// Main runs fn under the process-wide catalog and writes the report: to HAIRSPRING_EVENTS_FILE when it names a file
// that can be opened, otherwise to standard error, one write call per line, so concurrent writers to a pipe cannot
// interleave inside a line. Use it as os.Exit(hairspring.Main(run)). It also exports HAIRSPRING_PROC and
// HAIRSPRING_PROC_PID so that child processes derive their own tags.
func Main(fn func() int) int {
	env := processEnv{
		getenv: os.Getenv, stderr: privateStderr(), privileged: isPrivileged(), violationOnly: violationOnlyBuild,
		pid: os.Getpid(), setenv: func(k, v string) { _ = os.Setenv(k, v) }, // Setenv fails only on a bad key
	}
	return runProcess(std, env, fn)
}

func runProcess(c *Catalog, p processEnv, fn func() int) int {
	path := ""
	if !p.violationOnly { // the legacy markers belong on the output streams, not in a file
		path = eventsFilePath(p.getenv, p.privileged)
	}
	s := openSink(path, p.stderr)
	defer func() { _ = s.close() }() // every line was already written whole; a close error cannot lose one
	cfg := configFromEnv(p.getenv, p.pid, s.write, p.violationOnly)
	cfg.Dropped += s.dropped
	announced := !cfg.Disabled && !cfg.ViolationOnly // Run sends the hello before the workload can start a child
	p.setenv("HAIRSPRING_PROC", cfg.Proc)
	p.setenv("HAIRSPRING_PROC_PID", ownerValue(p.pid, announced, cfg.Unstable))
	return Run(c, cfg, fn)
}
