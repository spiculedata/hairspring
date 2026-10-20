//go:build !locked

package main

import "github.com/spiculedata/hairspring/examples/language-fixtures/go/internal/race"

// program is the racy build: no lock around the read-modify-write.
func program() race.Outcome { return race.Racy() }
