//go:build unix

package hairspring

import (
	"bytes"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// TestMain doubles as the child program of the closed-stderr test.
func TestMain(m *testing.M) {
	switch os.Getenv("HS_TEST_CHILD") {
	case "closed-stderr":
		h := Always("child.site", "runs in a child process")
		os.Exit(Main(func() int { h.Check(true, Vals{}); return 3 }))
	case "low-descriptors-closed":
		os.Exit(lowDescriptorsChild())
	case "stderr-closed":
		os.Exit(stderrClosedChild())
	}
	os.Exit(m.Run())
}

// lowDescriptorsChild closes fd 0 and fd 1 so that dup of fd 2 would return 0 (or 1), then reports through its exit
// status whether the private copy landed above 2. Exit 0 is success.
func lowDescriptorsChild() int {
	if syscall.Close(0) != nil || syscall.Close(1) != nil {
		return 10
	}
	f, err := privateCopy(2)
	if err != nil {
		return 11
	}
	if f.Fd() <= 2 {
		return 12
	}
	if _, err := f.Write([]byte("")); err != nil {
		return 13
	}
	return 0
}

// stderrClosedChild closes fd 2 and reports whether privateStderr falls back to os.Stderr instead of a bad copy.
func stderrClosedChild() int {
	if syscall.Close(2) != nil {
		return 10
	}
	if privateStderr() != io.Writer(os.Stderr) {
		return 11
	}
	return 0
}

func runChild(t *testing.T, mode string) int {
	t.Helper()
	cmd := exec.Command(os.Args[0])
	cmd.Env = append(os.Environ(), "HS_TEST_CHILD="+mode)
	err := cmd.Run()
	var exit *exec.ExitError
	if err == nil {
		return 0
	}
	if !errors.As(err, &exit) {
		t.Fatalf("child did not run: %v", err)
	}
	return exit.ExitCode()
}

func TestPrivateCopyNeverLandsOnFD0To2WhenLowDescriptorsAreClosed(t *testing.T) {
	// 11: privateCopy gave up; 12: the copy is on 0, 1 or 2.
	if code := runChild(t, "low-descriptors-closed"); code != 0 {
		t.Fatalf("child exit %d, want 0 (a copy above fd 2 even when dup would return 0)", code)
	}
}

func TestPrivateStderrFallsBackToStandardErrorWhenFD2IsClosed(t *testing.T) {
	if code := runChild(t, "stderr-closed"); code != 0 {
		t.Fatalf("child exit %d, want 0", code)
	}
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
