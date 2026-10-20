package sqlcancel

import (
	"strings"
	"testing"
)

func TestParseArgsDefaultsDependOnTheMode(t *testing.T) {
	for _, c := range []struct {
		args []string
		want Config
	}{
		{nil, Config{Mode: ModeStd, PerWorker: 1000}},
		{[]string{"-mode=std"}, Config{Mode: ModeStd, PerWorker: 1000}},
		{[]string{"-mode=yield"}, Config{Mode: ModeYield, PerWorker: 100}},
		{[]string{"-mode", "yield", "-q", "7"}, Config{Mode: ModeYield, PerWorker: 7}},
		{[]string{"-q=250"}, Config{Mode: ModeStd, PerWorker: 250}},
	} {
		got, err := ParseArgs(c.args)
		if err != nil || got != c.want {
			t.Errorf("ParseArgs(%v) = %+v, %v; want %+v", c.args, got, err, c.want)
		}
	}
}

func TestParseArgsRejectsBadInput(t *testing.T) {
	for name, args := range map[string][]string{
		"unknown mode": {"-mode=fast"},
		"zero q":       {"-q=0"},
		"negative q":   {"-q=-5"},
		"q too big":    {"-q=100001"},
		"not a number": {"-q=many"},
		"unknown flag": {"-seed=3"},
		"positional":   {"extra"},
		"empty mode":   {"-mode="},
	} {
		if _, err := ParseArgs(args); err == nil {
			t.Errorf("%s: expected an error", name)
		}
	}
}

func TestParseArgsErrorsNameTheProblem(t *testing.T) {
	_, err := ParseArgs([]string{"-mode=fast"})
	if err == nil || !strings.Contains(err.Error(), "fast") {
		t.Fatalf("error = %v", err)
	}
}

func TestExitStatusIsThreeOnlyWhenAHarnessInvariantFailed(t *testing.T) {
	if got := (Result{}).ExitStatus(); got != 0 {
		t.Errorf("clean run exits %d", got)
	}
	if got := (Result{HarnessFailures: 2}).ExitStatus(); got != ExitHarnessFailure {
		t.Errorf("harness failure exits %d", got)
	}
	// A violated property is reported only through the protocol; it never changes the exit status.
	if got := (Result{Summary: Summary{Torn: 5, ForeignHead: 5}}).ExitStatus(); got != 0 {
		t.Errorf("property violation exits %d, want 0", got)
	}
}

func TestSummaryLineHasFixedLengthWhateverTheCounts(t *testing.T) {
	small := Summary{Mode: ModeStd, Queries: 4000}.Line()
	big := Summary{Mode: ModeStd, Queries: 4000, ScanOK: 3999, ScanErrors: 1, ForeignHead: 99999999,
		Torn: 12, Unparsable: 3, Overlapped: 12345, ForeignNext: 1}.Line()

	if len(small) != len(big) {
		t.Fatalf("lengths differ: %d vs %d\n%q\n%q", len(small), len(big), small, big)
	}
	if !strings.HasSuffix(small, "\n") || strings.Count(small, "\n") != 1 {
		t.Fatalf("not exactly one line: %q", small)
	}
	want := "sqlcancel mode=std queries=    4000 scan_ok=       0 scan_err=       0 foreign_head=       0 torn=       0 unparsable=       0 overlapped=       0 foreign_next=       0\n"
	if small != want {
		t.Fatalf("got  %q\nwant %q", small, want)
	}
}
