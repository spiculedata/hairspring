// Command lost_update is the Go lost-update fixture: four goroutines make an unlocked read-modify-write on one
// counter (or, in the build with -tags locked, a locked one) and the program reports whether any update was lost.
//
// Property: go.counter_exact (always): the final counter equals workers * iterations. Coverage: go.all_started.
// Output: one fixed-width line on stdout. Exit status: 0, whatever the property says.
//
// Run it with GOMAXPROCS=2 and CGO_ENABLED=0, as the language acceptance plan does.
package main

import (
	"fmt"
	"math/rand/v2"
	"os"

	"github.com/spiculedata/hairspring/examples/language-fixtures/go/internal/race"
	hs "github.com/spiculedata/hairspring/sdks/go"
)

var (
	allStarted   = hs.Sometimes("go.all_started", "every worker started")
	counterExact = hs.Always("go.counter_exact", "no update was lost")
)

func main() { os.Exit(hs.Main(run)) }

func run() int {
	outcome := program()
	allStarted.Check(outcome.Started == race.Workers, hs.Vals{int64(outcome.Started)})
	counterExact.Check(outcome.Final == race.Expected, hs.Vals{int64(outcome.Final), race.Expected})
	fmt.Print(race.Line("go", outcome, rand.Uint64()))
	return 0
}
