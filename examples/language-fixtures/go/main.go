// Command at-go is the Go lost-update fixture: four goroutines make a read-modify-write on one counter, with a lock
// (fixed) or without one (buggy, wide), and the program reports whether any update was lost.
//
//	at-go buggy|fixed|wide
//
// Property: go.counter_exact (always): the final counter equals workers * iterations, with the witness values
// [final, 4000, 4, 1000]. Coverage: go.all_started (sometimes). Each is evaluated once, in main, after the workers
// are joined.
// Output: one contract line on stdout. Exit status: 0 whatever the property says, 3 for an internal invariant
// failure (a worker did not record its finish, or the line could not be written), 64 for a usage error.
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

const (
	exitInvariant = 3
	exitUsage     = 64
)

var (
	counterExact = hs.Always("go.counter_exact", "no update was lost")
	allStarted   = hs.Sometimes("go.all_started", "every worker started")
)

func main() {
	mode, err := race.ParseMode(os.Args[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(exitUsage)
	}
	os.Exit(hs.Main(func() int { return run(mode) }))
}

func run(mode race.Mode) int {
	outcome := race.Run(mode)
	if outcome.Arrived != race.Workers {
		fmt.Fprintf(os.Stderr, "at-go: %d of %d workers recorded their finish\n", outcome.Arrived, race.Workers)
		return exitInvariant
	}
	counterExact.Check(outcome.Final == race.Expected, hs.Vals{int64(outcome.Final), race.Expected, race.Workers, race.Iterations})
	allStarted.Check(outcome.Started == race.Workers, hs.Vals{int64(outcome.Started)})
	if _, err := os.Stdout.WriteString(race.Line("go", mode, outcome, rand.Uint64())); err != nil {
		fmt.Fprintf(os.Stderr, "at-go: cannot write the result line: %v\n", err)
		return exitInvariant
	}
	return 0
}
