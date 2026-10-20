// Command hssample is the conformance fixture for the Go SDK: one small program per scenario, whose report the Rust
// reference parser folds in crates/hairspring-props/tests/go_sdk.rs. Usage: hssample <scenario>.
package main

import (
	"fmt"
	"os"
	"strings"
	"sync"

	hs "github.com/spiculedata/hairspring/sdks/go"
)

type scenario func() func() int

var scenarios = map[string]scenario{
	"basic":      basic,
	"merge":      merge,
	"concurrent": concurrent,
	"exit3":      exit3,
	"manyids":    manyIDs,
	"longtext":   longText,
	"unreach":    unreach,
	"located":    located,
	"empty":      func() func() int { return func() int { return 0 } },
}

func main() {
	build, ok := scenarios[os.Getenv("HS_SCENARIO")]
	if !ok && len(os.Args) > 1 {
		build, ok = scenarios[os.Args[1]]
	}
	if !ok {
		fmt.Fprintln(os.Stderr, "usage: hssample <scenario>; scenarios are in main.go")
		os.Exit(2)
	}
	os.Exit(hs.Main(build()))
}

func basic() func() int {
	alwaysOK := hs.Declare(hs.Site{ID: "basic.always.ok", Kind: hs.KindAlways, Msg: "holds"})
	alwaysBad := hs.Declare(hs.Site{ID: "basic.always.bad", Kind: hs.KindAlways, Msg: "row mixed up", File: "main.go", Line: 40, Func: "basic"})
	sometimes := hs.Declare(hs.Site{ID: "basic.sometimes", Kind: hs.KindSometimes})
	reachHit := hs.Declare(hs.Site{ID: "basic.reach.hit", Kind: hs.KindReachable})
	hs.Declare(hs.Site{ID: "basic.reach.never", Kind: hs.KindReachable})
	hs.Declare(hs.Site{ID: "basic.unreach.clean", Kind: hs.KindUnreachable})
	hs.Declare(hs.Site{ID: "basic.aou.never", Kind: hs.KindAlwaysOrUnreachable})
	return func() int {
		for i := int64(0); i < 5; i++ {
			alwaysOK.Check(true, hs.Vals{i, 1, 0, 0})
			sometimes.Check(i == 3, hs.Vals{i, 0, 0, 0})
		}
		alwaysBad.Check(true, hs.Vals{})
		alwaysBad.Check(false, hs.Vals{7, 8, 0, 0})
		reachHit.Hit(hs.Vals{-1, 0, 0, 9223372036854775807})
		hs.AddWork(300)
		return 0
	}
}

func merge() func() int {
	a := hs.Declare(hs.Site{ID: "merge.shared", Kind: hs.KindAlways})
	b := hs.Declare(hs.Site{ID: "merge.shared", Kind: hs.KindAlways})
	return func() int {
		a.Check(true, hs.Vals{})
		a.Check(true, hs.Vals{})
		b.Check(false, hs.Vals{1, 2, 3, 4})
		return 0
	}
}

func concurrent() func() int {
	h := hs.Declare(hs.Site{ID: "conc.count", Kind: hs.KindAlways})
	const goroutines, each = 8, 1000
	return func() int {
		var wg sync.WaitGroup
		for g := int64(0); g < goroutines; g++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for i := int64(0); i < each; i++ {
					h.Check(i != 500, hs.Vals{g, i, 0, 0})
				}
			}()
		}
		wg.Wait()
		hs.AddWork(goroutines * each)
		return 0
	}
}

func exit3() func() int {
	h := hs.Declare(hs.Site{ID: "exit3.site", Kind: hs.KindReachable})
	return func() int { h.Hit(hs.Vals{}); return 3 }
}

func manyIDs() func() int {
	const n = 500
	handles := make([]*hs.Handle, n)
	for i := range handles {
		handles[i] = hs.Declare(hs.Site{ID: fmt.Sprintf("many.site.%04d", i), Kind: hs.KindAlways, Msg: "one of many"})
	}
	return func() int {
		for _, h := range handles {
			h.Check(true, hs.Vals{})
		}
		return 0
	}
}

func longText() func() int {
	h := hs.Declare(hs.Site{ID: "long.text", Kind: hs.KindAlways, Msg: strings.Repeat("<", 512), File: strings.Repeat("<", 256), Func: "f"})
	return func() int { h.Check(true, hs.Vals{}); return 0 }
}

func unreach() func() int {
	h := hs.Declare(hs.Site{ID: "unreach.hit", Kind: hs.KindUnreachable, Msg: "must not happen"})
	return func() int { h.Hit(hs.Vals{4, 0, 0, 0}); return 1 }
}

// located uses the constructors that record the caller's file and line.
func located() func() int {
	always := hs.Always("located.always", "holds")
	sometimes := hs.Sometimes("located.sometimes", "")
	reach := hs.Reachable("located.reach", "")
	return func() int {
		always.Check(true, hs.Vals{1})
		sometimes.Check(true, hs.Vals{})
		reach.Hit(hs.Vals{})
		return 0
	}
}
