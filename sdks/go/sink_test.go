package hairspring

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func env(m map[string]string) func(string) string {
	return func(k string) string { return m[k] }
}

func TestEventsFilePathReadsTheVariable(t *testing.T) {
	cases := []struct {
		name       string
		env        map[string]string
		privileged bool
		want       string
	}{
		{"unset", nil, false, ""},
		{"set", map[string]string{"HAIRSPRING_EVENTS_FILE": "/tmp/ev"}, false, "/tmp/ev"},
		{"events off", map[string]string{"HAIRSPRING_EVENTS_FILE": "/tmp/ev", "HAIRSPRING_EVENTS": "off"}, false, ""},
		{"set-id process", map[string]string{"HAIRSPRING_EVENTS_FILE": "/tmp/ev"}, true, ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := eventsFilePath(env(c.env), c.privileged); got != c.want {
				t.Fatalf("eventsFilePath = %q, want %q", got, c.want)
			}
		})
	}
}

func TestOpenSinkWithoutAPathWritesToStandardError(t *testing.T) {
	var stderr bytes.Buffer

	s := openSink("", &stderr)
	err := s.write([]byte("one\n"))

	if err != nil || stderr.String() != "one\n" || s.dropped != 0 {
		t.Fatalf("err=%v stderr=%q dropped=%d", err, stderr.String(), s.dropped)
	}
}

func TestOpenSinkAppendsToTheNamedFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events")
	if err := os.WriteFile(path, []byte("earlier\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer

	s := openSink(path, &stderr)
	err1, err2 := s.write([]byte("a\n")), s.write([]byte("b\n"))
	if err := s.close(); err != nil {
		t.Fatalf("close = %v", err)
	}

	got, _ := os.ReadFile(path)
	if err1 != nil || err2 != nil || string(got) != "earlier\na\nb\n" || stderr.Len() != 0 || s.dropped != 0 {
		t.Fatalf("file=%q stderr=%q dropped=%d err=%v %v", got, stderr.String(), s.dropped, err1, err2)
	}
}

func TestOpenSinkCreatesAMissingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "new-events")

	s := openSink(path, &bytes.Buffer{})
	_ = s.write([]byte("x\n"))
	_ = s.close()

	if got, _ := os.ReadFile(path); string(got) != "x\n" {
		t.Fatalf("file = %q", got)
	}
}

func TestOpenSinkFallsBackToStandardErrorAndCountsAFileThatCannotBeOpened(t *testing.T) {
	missingDir := filepath.Join(t.TempDir(), "no", "such", "dir", "events")
	var stderr bytes.Buffer

	s := openSink(missingDir, &stderr)
	_ = s.write([]byte("still reported\n"))

	if stderr.String() != "still reported\n" || s.dropped != 1 {
		t.Fatalf("stderr=%q dropped=%d, want the line on stderr and 1 dropped", stderr.String(), s.dropped)
	}
}

func TestOpenSinkRefusesASymbolicLink(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "target")
	link := filepath.Join(dir, "link")
	if err := os.WriteFile(target, []byte("keep\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symbolic links are not available: %v", err)
	}
	var stderr bytes.Buffer

	s := openSink(link, &stderr)
	_ = s.write([]byte("x\n"))

	if got, _ := os.ReadFile(target); string(got) != "keep\n" {
		t.Fatalf("the link target was written: %q", got)
	}
	if stderr.String() != "x\n" || s.dropped != 1 {
		t.Fatalf("stderr=%q dropped=%d", stderr.String(), s.dropped)
	}
}

func TestOpenSinkCloseOnTheStandardErrorSinkDoesNothing(t *testing.T) {
	s := openSink("", &bytes.Buffer{})

	if err := s.close(); err != nil {
		t.Fatalf("close = %v", err)
	}
}

func process(m map[string]string, stderr *bytes.Buffer, violationOnly bool) processEnv {
	return processEnv{getenv: env(m), stderr: stderr, violationOnly: violationOnly, pid: testPID, setenv: func(string, string) {}}
}

func TestRunProcessExportsTheTagForChildProcesses(t *testing.T) {
	cases := []struct {
		name         string
		env          map[string]string
		violation    bool
		wantTag      string
		wantOwner    string
		wantUnstable bool
	}{
		{"root, announced", nil, false, "p1", "4242:a", false},
		{"spawned child, announced and unstable", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "1"}, false, "p1.s4242", "4242:au", true},
		{"events off, nothing announced", map[string]string{"HAIRSPRING_EVENTS": "off"}, false, "p1", "4242", false},
		{"violation-only, nothing announced", nil, true, "p1", "4242", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			exported := map[string]string{}
			var stderr bytes.Buffer
			p := process(c.env, &stderr, c.violation)
			p.setenv = func(k, v string) { exported[k] = v }
			cat, fn := program(t)

			runProcess(cat, p, fn)

			if exported["HAIRSPRING_PROC"] != c.wantTag || exported["HAIRSPRING_PROC_PID"] != c.wantOwner {
				t.Fatalf("exported %v, want PROC=%s PROC_PID=%s", exported, c.wantTag, c.wantOwner)
			}
			if c.wantUnstable && !strings.Contains(stderr.String(), `"proc":"p1.s4242"`) {
				t.Fatalf("hello does not carry the derived tag: %s", stderr.String())
			}
			if c.wantUnstable && !strings.Contains(stderr.String(), `"tag_stable":false`) {
				t.Fatalf("hello does not say the tag is unstable: %s", stderr.String())
			}
		})
	}
}

func TestRunProcessCountsARefusedTagAndAnUnopenableFileTogether(t *testing.T) {
	var stderr bytes.Buffer
	missing := filepath.Join(t.TempDir(), "no", "dir", "events")
	p := process(map[string]string{"HAIRSPRING_PROC": "bad tag", "HAIRSPRING_EVENTS_FILE": missing}, &stderr, false)
	cat, fn := program(t)

	runProcess(cat, p, fn)

	lines := strings.Split(strings.TrimSuffix(stderr.String(), "\n"), "\n")
	if end := decode(t, []byte(lines[len(lines)-1]+"\n")); end["dropped"] != float64(2) {
		t.Fatalf("end = %v, want dropped=2", end)
	}
}

func TestRunProcessWritesTheReportToTheEventsFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events")
	c, fn := program(t)
	var stderr bytes.Buffer

	status := runProcess(c, process(map[string]string{"HAIRSPRING_EVENTS_FILE": path}, &stderr, false), fn)

	body, _ := os.ReadFile(path)
	lines := strings.SplitAfter(strings.TrimSuffix(string(body), "\n"), "\n")
	if status != 3 || stderr.Len() != 0 || len(lines) != 7 {
		t.Fatalf("status=%d stderr=%q lines=%d", status, stderr.String(), len(lines))
	}
}

func TestRunProcessReportsAnUnopenableEventsFileOnStandardError(t *testing.T) {
	c, fn := program(t)
	var stderr bytes.Buffer
	missing := filepath.Join(t.TempDir(), "no", "dir", "events")

	runProcess(c, process(map[string]string{"HAIRSPRING_EVENTS_FILE": missing}, &stderr, false), fn)

	lines := strings.Split(strings.TrimSuffix(stderr.String(), "\n"), "\n")
	end := decode(t, []byte(lines[len(lines)-1]+"\n"))
	if len(lines) != 7 || end["dropped"] != float64(1) {
		t.Fatalf("lines=%d end=%v", len(lines), end)
	}
}

func TestRunProcessViolationOnlyKeepsTheMarkersOnStandardError(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events")
	c, fn := program(t)
	var stderr bytes.Buffer

	runProcess(c, process(map[string]string{"HAIRSPRING_EVENTS_FILE": path}, &stderr, true), fn)

	if _, err := os.Stat(path); err == nil {
		t.Error("the violation-only build created the events file")
	}
	if stderr.String() != "HAIRSPRING_VIOLATION: r.bad: went wrong (1 false)\n" {
		t.Fatalf("stderr = %q", stderr.String())
	}
}

func TestRunProcessEventsOffWritesNothingAnywhere(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events")
	c, fn := program(t)
	var stderr bytes.Buffer

	runProcess(c, process(map[string]string{"HAIRSPRING_EVENTS_FILE": path, "HAIRSPRING_EVENTS": "off"}, &stderr, false), fn)

	if _, err := os.Stat(path); err == nil || stderr.Len() != 0 {
		t.Fatalf("events off still wrote: stat err=%v stderr=%q", err, stderr.String())
	}
}
