package race

import (
	"fmt"
	"hash/fnv"
	"regexp"
	"runtime"
	"sort"
	"testing"

	hs "github.com/spiculedata/hairspring/sdks/go"
)

const nativeRuns = 50

func TestSpinDependsOnItsSeedAndItsRounds(t *testing.T) {
	if spin(7, 0) != 7 {
		t.Fatal("zero rounds must return the seed")
	}
	if spin(7, 10) == spin(7, 11) || spin(7, 10) == spin(8, 10) {
		t.Fatal("the busy loop did not mix its seed and rounds")
	}
}

func TestOrderHashIsFNV1aOverTheArrivalOrder(t *testing.T) {
	want := fnv.New64a()
	want.Write([]byte("2031"))

	got := orderHash([]uint8("2031"))

	if got != want.Sum64() {
		t.Fatalf("orderHash = %016x, want %016x", got, want.Sum64())
	}
	if orderHash([]uint8("a")) != 0xaf63dc4c8601ec8c {
		t.Fatalf("orderHash(a) = %016x, want the published FNV-1a vector af63dc4c8601ec8c", orderHash([]uint8("a")))
	}
}

func TestTheContractConstantsAreTheSameForEveryLanguage(t *testing.T) {
	if Workers != 4 || Iterations != 1000 || Spin != 1000 || SpinWide != 10000 || Expected != 4000 {
		t.Fatalf("T=%d K=%d W=%d W_wide=%d expected=%d, want 4 1000 1000 10000 4000", Workers, Iterations, Spin, SpinWide, Expected)
	}
}

func TestParseModeAcceptsExactlyOneOfTheThreeModes(t *testing.T) {
	cases := []struct {
		args []string
		want Mode
	}{
		{[]string{"buggy"}, Mode{Name: "buggy", Locked: false, Spin: 1000}},
		{[]string{"fixed"}, Mode{Name: "fixed", Locked: true, Spin: 1000}},
		{[]string{"wide"}, Mode{Name: "wide", Locked: false, Spin: 10000}},
	}
	for _, c := range cases {
		got, err := ParseMode(c.args)

		if err != nil || got != c.want {
			t.Errorf("ParseMode(%v) = %+v, %v; want %+v", c.args, got, err, c.want)
		}
	}
}

func TestParseModeRejectsEverythingElseWithAUsageError(t *testing.T) {
	for _, args := range [][]string{nil, {}, {"Buggy"}, {"racy"}, {"locked"}, {""}, {"buggy", "extra"}, {"--switch=1.0"}} {
		_, err := ParseMode(args)

		if err == nil {
			t.Errorf("ParseMode(%q) accepted its arguments", args)
		}
	}
}

func TestWideDiffersFromBuggyOnlyInTheBusyLoopLength(t *testing.T) {
	buggy, _ := ParseMode([]string{"buggy"})
	wide, _ := ParseMode([]string{"wide"})

	wide.Name, wide.Spin = buggy.Name, buggy.Spin

	if wide != buggy {
		t.Fatalf("wide and buggy differ in more than the name and the spin: %+v vs %+v", wide, buggy)
	}
}

func TestLineIsTheContractLine(t *testing.T) {
	buggy := Mode{Name: "buggy"}
	quiet := Line("go", buggy, Outcome{Final: 0}, 0)
	busy := Line("go", buggy, Outcome{Final: Expected}, ^uint64(0))

	if len(quiet) != len(busy) {
		t.Fatalf("the line's width depends on its values: %d vs %d", len(quiet), len(busy))
	}
	shape := regexp.MustCompile(`^LANGAT v1 lang=go mode=buggy final=\d{20} expect=0{16}4000 order=[0-9a-f]{16} rand=[0-9a-f]{16}\n$`)
	if !shape.MatchString(busy) {
		t.Fatalf("line = %q", busy)
	}
	order := Outcome{Final: 3999, Order: [Workers]uint8{'0', '1', '2', '3'}}
	want := fmt.Sprintf("LANGAT v1 lang=go mode=wide final=00000000000000003999 expect=00000000000000004000 order=%016x rand=0000000000000001\n",
		orderHash([]uint8("0123")))
	if got := Line("go", Mode{Name: "wide"}, order, 1); got != want {
		t.Fatalf("line = %q, want %q", got, want)
	}
}

func TestFixedNeverLosesAnUpdate(t *testing.T) {
	for i := 0; i < nativeRuns; i++ {
		o := Run(Mode{Name: "fixed", Locked: true, Spin: Spin})

		if o.Final != Expected || o.Started != Workers {
			t.Fatalf("run %d: final=%d started=%d, want %d and %d", i, o.Final, o.Started, Expected, Workers)
		}
		if !isArrivalList(o) {
			t.Fatalf("run %d: arrival list %q is not each worker once", i, o.Order)
		}
	}
}

func TestBuggyLosesAnUpdateAtLeastOnce(t *testing.T) {
	skipWithoutParallelism(t)
	lost := 0
	for i := 0; i < nativeRuns; i++ {
		o := Run(Mode{Name: "buggy", Spin: Spin})
		if o.Final > Expected {
			t.Fatalf("run %d: final=%d is above %d: the counter cannot gain updates", i, o.Final, Expected)
		}
		if o.Final < Expected {
			lost++
		}
	}
	if lost == 0 {
		t.Fatalf("no update was lost in %d runs: the planted race is not reachable here", nativeRuns)
	}
}

func TestWideRunsTheSameProgramWithALongerWindow(t *testing.T) {
	o := Run(Mode{Name: "wide", Spin: SpinWide})

	if o.Final > Expected || o.Started != Workers || !isArrivalList(o) {
		t.Fatalf("wide run: final=%d started=%d order=%q", o.Final, o.Started, o.Order)
	}
}

func TestEveryWorkerPassesTheStartBarrierBeforeItsFirstIteration(t *testing.T) {
	for i := 0; i < nativeRuns; i++ {
		o := Run(Mode{Name: "buggy", Spin: Spin})

		for id, seen := range o.StartedAtFirstIteration {
			if seen != Workers {
				t.Fatalf("run %d: worker %d began iterating when %d of %d workers had started", i, id, seen, Workers)
			}
		}
	}
}

// The property as the fixture reports it, driven natively with its four witness values: it must be false for the
// buggy mode and never for the fixed one.
func TestTheDetectorFiresOnTheBuggyModeAndNeverOnTheFixedOne(t *testing.T) {
	skipWithoutParallelism(t)
	fired := func(mode Mode) int {
		n := 0
		for i := 0; i < nativeRuns; i++ {
			c := hs.NewCatalog()
			h, err := c.Declare(hs.Site{ID: "go.counter_exact", Kind: hs.KindAlways})
			if err != nil {
				t.Fatal(err)
			}
			o := Run(mode)
			h.Check(o.Final == Expected, hs.Vals{int64(o.Final), Expected, Workers, Iterations})
			if len(c.Snapshot().Violations()) > 0 {
				n++
			}
		}
		return n
	}

	buggy, fixed := fired(Mode{Name: "buggy", Spin: Spin}), fired(Mode{Name: "fixed", Locked: true, Spin: Spin})

	if buggy == 0 {
		t.Errorf("the detector never fired on the buggy mode in %d runs", nativeRuns)
	}
	if fixed != 0 {
		t.Errorf("the detector fired %d times on the fixed mode", fixed)
	}
	t.Logf("buggy fired in %d of %d runs, fixed in %d", buggy, nativeRuns, fixed)
}

func skipWithoutParallelism(t *testing.T) {
	t.Helper()
	if runtime.GOMAXPROCS(0) < 2 || runtime.NumCPU() < 2 {
		t.Skip("the planted race needs two goroutines running at once")
	}
}

// isArrivalList reports whether the outcome's arrival list holds each worker's digit exactly once.
func isArrivalList(o Outcome) bool {
	sorted := append([]uint8(nil), o.Order[:]...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i] < sorted[j] })
	for i, v := range sorted {
		if int(v) != '0'+i {
			return false
		}
	}
	return o.Arrived == Workers
}
