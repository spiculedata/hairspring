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
	want.Write([]byte{2, 0, 3, 1})

	got := orderHash([]uint8{2, 0, 3, 1})

	if got != want.Sum64() {
		t.Fatalf("orderHash = %016x, want %016x", got, want.Sum64())
	}
	if orderHash([]uint8("a")) != 0xaf63dc4c8601ec8c {
		t.Fatalf("orderHash(a) = %016x, want the published FNV-1a vector af63dc4c8601ec8c", orderHash([]uint8("a")))
	}
}

func TestLineIsFixedWidthAndMatchesTheCFormat(t *testing.T) {
	quiet := Line("go", Outcome{Final: 0}, 0)
	busy := Line("go", Outcome{Final: Expected}, ^uint64(0))

	if len(quiet) != len(busy) {
		t.Fatalf("the line's width depends on its values: %d vs %d", len(quiet), len(busy))
	}
	shape := regexp.MustCompile(`^lang=go  final=\d{10} order=[0-9a-f]{16} rand=[0-9a-f]{16}\n$`)
	if !shape.MatchString(busy) {
		t.Fatalf("line = %q", busy)
	}
	zeroOrder := fnv.New64a()
	zeroOrder.Write([]byte{0, 0, 0, 0})
	want := fmt.Sprintf("lang=go  final=0000004000 order=%016x rand=0000000000000001\n", zeroOrder.Sum64())
	if got := Line("go", Outcome{Final: 4000}, 1); got != want {
		t.Fatalf("line = %q, want %q", got, want)
	}
}

func TestLockedNeverLosesAnUpdate(t *testing.T) {
	for i := 0; i < nativeRuns; i++ {
		o := Locked()

		if o.Final != Expected || o.Started != Workers {
			t.Fatalf("run %d: final=%d started=%d, want %d and %d", i, o.Final, o.Started, Expected, Workers)
		}
		if !isPermutation(o.Order[:]) {
			t.Fatalf("run %d: arrival order %v is not each worker once", i, o.Order)
		}
	}
}

func TestRacyLosesAnUpdateAtLeastOnce(t *testing.T) {
	skipWithoutParallelism(t)
	lost := 0
	for i := 0; i < nativeRuns; i++ {
		o := Racy()
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

// The property as the fixture reports it, driven natively: it must be false for the racy program and never for the locked one.
func TestTheDetectorFiresOnTheRacyProgramAndNeverOnTheLockedOne(t *testing.T) {
	skipWithoutParallelism(t)
	fired := func(program func() Outcome) int {
		n := 0
		for i := 0; i < nativeRuns; i++ {
			c := hs.NewCatalog()
			h, err := c.Declare(hs.Site{ID: "go.counter_exact", Kind: hs.KindAlways})
			if err != nil {
				t.Fatal(err)
			}
			o := program()
			h.Check(o.Final == Expected, hs.Vals{int64(o.Final), Expected})
			if len(c.Snapshot().Violations()) > 0 {
				n++
			}
		}
		return n
	}

	racy, locked := fired(Racy), fired(Locked)

	if racy == 0 {
		t.Errorf("the detector never fired on the racy program in %d runs", nativeRuns)
	}
	if locked != 0 {
		t.Errorf("the detector fired %d times on the locked program", locked)
	}
	t.Logf("racy fired in %d of %d runs, locked in %d", racy, nativeRuns, locked)
}

func skipWithoutParallelism(t *testing.T) {
	t.Helper()
	if runtime.GOMAXPROCS(0) < 2 || runtime.NumCPU() < 2 {
		t.Skip("the planted race needs two goroutines running at once")
	}
}

func isPermutation(order []uint8) bool {
	sorted := append([]uint8(nil), order...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i] < sorted[j] })
	for i, v := range sorted {
		if int(v) != i {
			return false
		}
	}
	return len(sorted) == Workers
}
