//go:build unix

package hairspring

import (
	"bytes"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// TestMain doubles as the child program of the closed-stderr test.
func TestMain(m *testing.M) {
	if os.Getenv("HS_TEST_CHILD") == "closed-stderr" {
		h := Always("child.site", "runs in a child process")
		os.Exit(Main(func() int { h.Check(true, Vals{}); return 3 }))
	}
	os.Exit(m.Run())
}

// By default Go ends a program that writes to a closed pipe on fd 1 or 2 with SIGPIPE. A report to a closed reader
// must cost the report, not the program's exit status.
func TestAClosedStandardErrorDoesNotChangeTheExitStatus(t *testing.T) {
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	_ = r.Close()
	cmd := exec.Command(os.Args[0])
	cmd.Env = append(os.Environ(), "HS_TEST_CHILD=closed-stderr", "HAIRSPRING_EVENTS_FILE=", "HAIRSPRING_PROC=")
	cmd.Stderr = w

	runErr := cmd.Run()
	_ = w.Close()

	var exit *exec.ExitError
	if !errors.As(runErr, &exit) || exit.ExitCode() != 3 {
		t.Fatalf("child result = %v, want exit status 3 (a signal death shows as -1)", runErr)
	}
}

func TestPrivateStderrSurvivesTheProgramClosingFD2(t *testing.T) {
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()

	out, err := privateCopy(int(w.Fd()))
	if err != nil {
		t.Fatal(err)
	}
	_ = w.Close()
	_, writeErr := out.Write([]byte("kept\n"))

	buf := make([]byte, 5)
	n, _ := r.Read(buf)
	if writeErr != nil || string(buf[:n]) != "kept\n" {
		t.Fatalf("write err=%v read=%q", writeErr, buf[:n])
	}
}

func openSinkWithin(t *testing.T, path string, stderr *bytes.Buffer) eventSink {
	t.Helper()
	done := make(chan eventSink, 1)
	go func() { done <- openSink(path, stderr) }()
	select {
	case s := <-done:
		return s
	case <-time.After(5 * time.Second):
		t.Fatalf("openSink(%s) hung", path)
		return eventSink{}
	}
}

func TestOpenSinkRefusesAFIFOWithoutBlocking(t *testing.T) {
	fifo := filepath.Join(t.TempDir(), "events")
	if err := syscall.Mkfifo(fifo, 0o600); err != nil {
		t.Skipf("no FIFOs here: %v", err)
	}
	var stderr bytes.Buffer

	// No reader: a plain open for writing would block until one appears.
	s := openSinkWithin(t, fifo, &stderr)
	_ = s.write([]byte("x\n"))

	if stderr.String() != "x\n" || s.dropped != 1 {
		t.Fatalf("no reader: stderr=%q dropped=%d", stderr.String(), s.dropped)
	}
}

func TestOpenSinkRefusesAFIFOThatHasAReader(t *testing.T) {
	fifo := filepath.Join(t.TempDir(), "events")
	if err := syscall.Mkfifo(fifo, 0o600); err != nil {
		t.Skipf("no FIFOs here: %v", err)
	}
	reader := make(chan *os.File, 1)
	go func() { f, _ := os.Open(fifo); reader <- f }() // blocks until a writer opens the FIFO
	time.Sleep(100 * time.Millisecond)
	var stderr bytes.Buffer

	s := openSinkWithin(t, fifo, &stderr)
	_ = s.write([]byte("x\n"))

	if f := <-reader; f != nil {
		_ = f.Close()
	}
	if stderr.String() != "x\n" || s.dropped != 1 {
		t.Fatalf("with a reader: stderr=%q dropped=%d, want the FIFO refused (it is not a regular file)", stderr.String(), s.dropped)
	}
}

func TestOpenSinkRefusesADevice(t *testing.T) {
	var stderr bytes.Buffer

	s := openSinkWithin(t, "/dev/null", &stderr)

	if s.dropped != 1 {
		t.Fatalf("dropped = %d, want /dev/null refused as not a regular file", s.dropped)
	}
}
