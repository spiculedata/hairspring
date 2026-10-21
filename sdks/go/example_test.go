package hairspring_test

import (
	"fmt"
	"os"
	"strings"
	"sync"

	hairspring "github.com/spiculedata/hairspring/sdks/go"
)

// Run a small workload under a private catalog and print what the report says about one property. A real program
// declares its sites at package level and calls hairspring.Main(run); Run is the same thing with the catalog and the
// destination given explicitly.
func Example() {
	catalog := hairspring.NewCatalog()
	site, err := catalog.Declare(hairspring.Site{ID: "demo.total_exact", Kind: hairspring.KindAlways, Msg: "no increment may be lost"})
	if err != nil {
		panic(err)
	}
	var lines []string
	cfg := hairspring.Config{Proc: "p1", Write: func(l []byte) error { lines = append(lines, string(l)); return nil }}

	status := hairspring.Run(catalog, cfg, func() int {
		var mu sync.Mutex
		total := 0
		var wg sync.WaitGroup
		for w := 0; w < 4; w++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for i := 0; i < 100; i++ {
					mu.Lock()
					total++
					mu.Unlock()
				}
			}()
		}
		wg.Wait()
		site.Check(total == 400, hairspring.Vals{int64(total), 400})
		return 0
	})

	for _, l := range lines {
		if strings.Contains(l, `"kind":"assert"`) {
			fmt.Println(strings.Contains(l, `"falses":`+strings.Repeat(" ", 19)+"0"))
		}
	}
	fmt.Println("status", status)
	// Output:
	// true
	// status 0
}

// Main is the entry point of a program: it declares nothing itself, runs the workload, writes the report to standard
// error (or to HAIRSPRING_EVENTS_FILE) and returns the status to exit with.
func ExampleMain() {
	var reachedEnd = hairspring.Reachable("demo.reached_end", "the workload ran to completion")

	run := func() int {
		reachedEnd.Hit(hairspring.Vals{})
		return 0
	}

	if os.Getenv("HAIRSPRING_EXAMPLE_RUN") == "1" { // keep go test output clean
		os.Exit(hairspring.Main(run))
	}
}
