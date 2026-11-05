# Testing a service, not just a program

A `process` target is **one command that starts, does its work and exits with a verdict**. A server does not do that:
it starts and waits. To test a service, make the command a small **driver** that runs the service for you.

The driver:

1. starts your service as a child process;
2. waits for it to listen on a loopback port;
3. sends a fixed workload;
4. checks the result, or relies on properties the service declares through an SDK;
5. stops the service;
6. exits with a verdict.

Everything happens inside one Hermit trial, so the service's threads and the driver's are scheduled together from the
trial's seed. Each trial is one start, one workload, one stop.

The process backend needs Linux on x86-64 with hardware counters: see
[is my machine suitable?](is-my-machine-suitable.md). The only configuration that was measured is in the
[support matrix](../support-matrix.md), and the language you write the service in has its own row there.

**What was run for this page.** The driver and the service below were built and run natively on an Apple silicon Mac,
which is enough to prove the driver's logic. The Linux x86-64 builds were made (`file` says `statically linked`). Nothing
was run under Hermit: it needs Linux x86-64 with a PMU, and the machines used here had none. So you will see below that
the bug shows up natively, but whether, how often and at which seeds it shows up under Hermit was not measured here, and
neither was how Hermit handles loopback networking. The `hairspring.toml` entry was parsed by `hairspring targets`.

## The pattern

### What decides pass or fail

Hairspring looks at what the driver did. In the order it applies them (the full list is in the
[guest protocol](../guest-protocol.md) and the wiki's
[Running your own software](https://github.com/spiculedata/hairspring/wiki/Running-Your-Own-Software#how-a-failure-is-detected)):
a timeout is inconclusive (or a violation if the target sets `hang_is_violation`), a failed SDK property fails the
trial, a line starting `HAIRSPRING_VIOLATION:` on stdout or stderr fails it with that text as the reason, a non-zero exit
status fails it, and a clean exit passes it. So the driver needs only to exit non-zero, or print a violation line, when
the service misbehaved. If the service itself uses a guest SDK, its properties are reported too, with no extra work in
the driver: the SDK writes to standard error, the driver passes the child's standard error through, and Hairspring reads
it.

Keep the two failure kinds apart in your driver: exit 1 (or a violation line) for "the service is wrong", and a
different code, 2 below, for "the driver could not do its job" (the port never opened, the service would not start).
Otherwise an infrastructure problem looks like a bug in your service.

### Rules for the driver

- **Fixed port, fixed working directory.** Do not pick a random port or directory: trials should differ only in the
  scheduler seed. Put the port in the target's `env` and read it in the driver. Use `cwd` in the target for the working
  directory. A fixed port has a cost: if something else already listens there, the driver can talk to the wrong
  process. In the run below, with a different program already holding the port, the service failed to start and the
  driver's connections were reset by the other listener; the driver exited 2, but only after confusing output. Make the
  driver check that its own child is still alive before it trusts the port, if that matters to you.
- **Configuration comes from the target's `env`, not your shell.** Hairspring starts the program from a minimal pinned
  environment (`PATH=/usr/local/bin:/usr/bin:/bin`, `LANG=C`, `LC_ALL=C`, `TZ=UTC`) plus what you put in `env`, with an
  empty standard input. Anything you exported in your terminal does not reach the driver. The driver passes its own
  environment to the service (`os.Environ()` below), so a variable set in `env` reaches both.
- **Wait for readiness by connecting, with a bound.** Poll the port for a short, fixed total time and give up with an
  infrastructure exit code. Do not sleep for a guessed duration.
- **Stop the service on every exit path.** In the first version of the driver written for this page, one error path
  called `os.Exit` without stopping the child, and the service was left running. Keep one function that stops the child
  and call it before every exit.
- **Keep output free of the clock, pids and ephemeral ports** if you want `hairspring doctor` to report the target as
  deterministic. It compares each seed's exit status and output across two runs. The service below turns off the log
  timestamp for that reason.
- **Trial timeouts.** Set `trial_timeout_secs` for the whole start, workload and stop under Hermit, which is slower than
  native; a timeout is inconclusive, never a pass. Raise `wall_secs` in the target's budget, whose default of 60 seconds
  allows few Hermit trials.
- **Process groups.** When a trial times out or ends, Hairspring kills the program's whole process group, so the service
  does not outlive the trial. A child that calls `setsid` leaves the group and is not killed. Start the service in the
  same group as the driver, and do not daemonise it.
- **Parallelism.** Leave `parallelism` at its default of 1 while the port is fixed. Whether trials that run at once see
  each other's loopback ports under Hermit was not tested.
- **One command, absolute paths.** The service binary is the driver's argument, with an absolute path. For Java, keep the
  JDK, the jar and the working directory at the same paths for recording and replay (see
  [build a Linux executable](build-a-linux-executable.md)).

## A worked example in Go

A TCP service with a lost-update bug, and a driver for it. Both are Go, and the Go SDK snippet works natively on a Mac,
so everything but Hermit was run. The service holds a counter. A client sends `INCR`, the service reads the counter and
writes it back plus one, and replies. Two connections can interleave between the read and the write, so updates are
lost. (`runtime.Gosched()` between the read and the write widens the window so that a native run shows the bug; a real
service has a window of its own.) `SERVICE_FIXED=1` selects a corrected version that uses an atomic add.

The service, `service/main.go`:

```go
// A tiny TCP service with a lost-update bug. One line in, one line out:
//
//	INCR  adds one to a shared counter and replies with the new value
//	GET   replies with the counter
package main

import (
	"bufio"
	"fmt"
	"log"
	"net"
	"os"
	"runtime"
	"sync/atomic"
)

var (
	counter atomic.Int64
	fixed   = os.Getenv("SERVICE_FIXED") == "1" // set in the target's env to run the corrected service
)

func handle(conn net.Conn) {
	defer conn.Close()
	in := bufio.NewScanner(conn)
	for in.Scan() {
		switch in.Text() {
		case "INCR":
			if fixed {
				counter.Add(1)
			} else {
				v := counter.Load()
				runtime.Gosched() // widens the window so a native run shows the bug; a real service has its own window
				counter.Store(v + 1)
			}
			fmt.Fprintln(conn, counter.Load())
		case "GET":
			fmt.Fprintln(conn, counter.Load())
		}
	}
}

func main() {
	log.SetFlags(0) // no timestamps: a run's output should not depend on the clock
	addr := os.Getenv("SERVICE_ADDR")
	if addr == "" {
		log.Fatal("SERVICE_ADDR is not set")
	}
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		log.Fatal(err)
	}
	for {
		conn, err := ln.Accept()
		if err != nil {
			log.Fatal(err)
		}
		go handle(conn)
	}
}
```

The driver, `driver/main.go`. Its one argument is the service binary; the port comes from `SERVICE_PORT`:

```go
// The driver: the one command Hairspring runs. It starts the service, sends a fixed workload, checks the result,
// stops the service and exits with the verdict (0 = the service behaved, 1 = it did not, 2 = the driver itself failed).
package main

import (
	"bufio"
	"fmt"
	"net"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"time"
)

const (
	clients         = 4
	incrPerClient   = 50
	listenWait      = 10 * time.Second
	exitInfraFailed = 2
)

var service *exec.Cmd

// stopService ends the child. Every exit path must call it, or a failed driver leaves the service running.
func stopService() {
	if service != nil && service.Process != nil {
		service.Process.Signal(syscall.SIGTERM)
		service.Wait()
	}
}

func infra(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "driver: "+format+"\n", args...)
	stopService()
	os.Exit(exitInfraFailed)
}

func waitForPort(addr string) {
	deadline := time.Now().Add(listenWait)
	for time.Now().Before(deadline) {
		if c, err := net.Dial("tcp", addr); err == nil {
			c.Close()
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	infra("service did not listen on %s within %s", addr, listenWait)
}

func worker(addr string, wg *sync.WaitGroup) {
	defer wg.Done()
	c, err := net.Dial("tcp", addr)
	if err != nil {
		infra("dial: %v", err)
	}
	defer c.Close()
	r := bufio.NewReader(c)
	for i := 0; i < incrPerClient; i++ {
		fmt.Fprintln(c, "INCR")
		if _, err := r.ReadString('\n'); err != nil {
			infra("read: %v", err)
		}
	}
}

func finalCount(addr string) int {
	c, err := net.Dial("tcp", addr)
	if err != nil {
		infra("dial: %v", err)
	}
	defer c.Close()
	fmt.Fprintln(c, "GET")
	var n int
	if _, err := fmt.Fscanln(c, &n); err != nil {
		infra("read count: %v", err)
	}
	return n
}

func main() {
	if len(os.Args) != 2 {
		infra("usage: driver SERVICE_BINARY")
	}
	port := os.Getenv("SERVICE_PORT") // fixed, from the target's env, never chosen at random
	if port == "" {
		infra("SERVICE_PORT is not set")
	}
	addr := "127.0.0.1:" + port
	service = exec.Command(os.Args[1])
	service.Env = append(os.Environ(), "SERVICE_ADDR="+addr)
	service.Stdout, service.Stderr = os.Stderr, os.Stderr
	if err := service.Start(); err != nil {
		infra("start service: %v", err)
	}
	waitForPort(addr)

	var wg sync.WaitGroup
	for i := 0; i < clients; i++ {
		wg.Add(1)
		go worker(addr, &wg)
	}
	wg.Wait()

	got, want := finalCount(addr), clients*incrPerClient
	stopService()

	if got != want {
		fmt.Printf("HAIRSPRING_VIOLATION: lost updates: counter is %d, expected %d\n", got, want)
		os.Exit(1)
	}
	fmt.Printf("ok: counter is %d\n", got)
}
```

Build both natively, with a module of their own:

```bash
go mod init example.com/counter-test
go build -o service-bin ./service && go build -o driver-bin ./driver
```

### Prove the driver natively

This is the step that needs no Hermit. Run the driver by hand against the buggy service, then against the fixed one, and
against two broken setups. Run here on a Mac with Go 1.27.1:

```text
$ SERVICE_PORT=7391 ./driver-bin ./service-bin
HAIRSPRING_VIOLATION: lost updates: counter is 86, expected 200
$ echo $?
1
$ SERVICE_PORT=7391 ./driver-bin ./service-bin       # two more runs: counters 147 and 146, exit 1 each time
$ SERVICE_FIXED=1 SERVICE_PORT=7391 ./driver-bin ./service-bin
ok: counter is 200
$ echo $?
0
$ ./driver-bin ./service-bin                         # no SERVICE_PORT
driver: SERVICE_PORT is not set
$ echo $?
2
$ SERVICE_PORT=7391 ./driver-bin ./nope              # service binary missing
driver: start service: fork/exec ./nope: no such file or directory
$ echo $?
2
```

Natively the counter differs from run to run (86, 147 and 146 in these three runs). That is the
reason to use Hairspring: a native run is a fresh roll of the dice and a failure cannot be replayed, while a trial under
Hermit is meant to be reproducible from its seed. After the runs above no service process was left behind.

### Build for Linux and describe the target

Cross-build both programs for Linux (run here from the Mac; `file` reported `statically linked` for each):

```bash
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o driver-linux ./driver
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o service-linux ./service
```

Copy them to the Linux machine, for example into `/opt/counter-test`, and describe the target there. The second target
runs the fixed service, so you can compare:

```toml
[target.counter-service]
kind = "process"
description = "TCP counter service, started and exercised by a driver"
command = ["/opt/counter-test/driver-linux", "/opt/counter-test/service-linux"]
cwd = "/opt/counter-test"
env = { SERVICE_PORT = "7391" }
trial_timeout_secs = 120

[target.counter-service.budget]
max_trials = 50
wall_secs = 600

[target.counter-service-fixed]
kind = "process"
command = ["/opt/counter-test/driver-linux", "/opt/counter-test/service-linux"]
cwd = "/opt/counter-test"
env = { SERVICE_PORT = "7391", SERVICE_FIXED = "1" }
trial_timeout_secs = 120

[target.counter-service-fixed.budget]
max_trials = 50
wall_secs = 600
```

Check that the file parses (run here on the Mac; it needs no Hermit):

```text
$ hairspring targets
counter-service          process  TCP counter service, started and exercised by a driver
counter-service-fixed    process
```

On the Linux machine, with Hermit installed ([quickstart](../quickstart.md#install)), the next steps are the same as for
any program. They were **not run here**:

```bash
hairspring doctor counter-service            # is the driver plus service deterministic under Hermit?
hairspring test counter-service --seed 1     # explore; a failure writes a bundle
hairspring replay .hairspring/bundles/<bundle>
hairspring test counter-service-fixed --seed 1
```

A `doctor` that reports a seed where two runs differed means something in the output or the exit status depends on
more than the schedule. Fix that first. If the files are not at the paths in `command`, `doctor` says so; run here on
the Mac, where `/opt/counter-test` does not exist, it printed
`error: backend error: target counter-service: program /opt/counter-test/driver-linux not found or not executable`.

## Using an SDK in the service instead

The driver above checks one number. If the service is in a language that has a guest SDK, you can declare the property
in the service (`always`, `sometimes`, `reachable` and so on) and let the driver only start, wait, send load and stop.
The SDK reports at the service's exit, so the driver must stop the service in a way that lets it exit and report, such as
a termination signal the service handles, or a shutdown command, rather than a kill. The SDK documents describe what
happens at a signal ([Guest SDKs by language](https://github.com/spiculedata/hairspring/wiki/Guest-SDKs-by-Language)).
This combination was not tried here.

## Where to go next

- [Build a Linux executable](build-a-linux-executable.md) for the other languages.
- [Test your logic with wasm](test-logic-with-wasm.md) if you want to explore your own logic on a Mac, with no Hermit.
- [Running your own software](https://github.com/spiculedata/hairspring/wiki/Running-Your-Own-Software) for the keys of
  a `process` target, budgets, sweeps and confirmation.
