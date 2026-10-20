//go:build locked

package main

import "github.com/spiculedata/hairspring/examples/language-fixtures/go/internal/race"

// program is the locked twin, built with -tags locked: a mutex across the read-modify-write.
func program() race.Outcome { return race.Locked() }
