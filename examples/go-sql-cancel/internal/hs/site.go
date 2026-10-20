// Package hs is a small Go shim for guest event protocol v1 (docs/guest-protocol.md): it counts assertion
// evaluations without a system call and writes the report as HAIRSPRING_EVENT lines when the program ends.
// It is an example-internal package, not a published SDK. Limits: goroutines are not threads, so no thread
// lines are sent; os.Exit and a panic bypass the report (hello says signal_handlers=false).
package hs

import (
	"errors"
	"fmt"
	"regexp"
	"unicode"
	"unicode/utf8"
)

// SDKVersion is sent in the hello line.
const SDKVersion = "0.1.0"

// Kind is an assertion kind as named by the protocol.
type Kind string

const (
	KindAlways              Kind = "always"
	KindAlwaysOrUnreachable Kind = "always_or_unreachable"
	KindSometimes           Kind = "sometimes"
	KindReachable           Kind = "reachable"
	KindUnreachable         Kind = "unreachable"
)

func (k Kind) valid() bool {
	switch k {
	case KindAlways, KindAlwaysOrUnreachable, KindSometimes, KindReachable, KindUnreachable:
		return true
	}
	return false
}

func (k Kind) mustHit() bool { return k == KindAlways || k == KindSometimes || k == KindReachable }

// isPoint is true for the kinds that have no condition (reachable, unreachable).
func (k Kind) isPoint() bool { return k == KindReachable || k == KindUnreachable }

// Protocol limits that the shim enforces itself (bytes).
const (
	maxIDBytes   = 128
	maxMsgBytes  = 512
	maxFileBytes = 256
	maxFuncBytes = 128
)

var identPattern = regexp.MustCompile(`^[A-Za-z0-9._:/-]+$`)

// Site describes one assertion. Sites are plain values; the catalog never changes one after Declare.
type Site struct {
	ID   string
	Kind Kind
	Msg  string
	File string
	Line uint64
	Func string
}

// Validate reports why a site cannot be sent under protocol v1, or nil.
func (s Site) Validate() error {
	if len(s.ID) > maxIDBytes || !identPattern.MatchString(s.ID) {
		return fmt.Errorf("hs: invalid id %q (1-%d bytes of [A-Za-z0-9._:/-])", s.ID, maxIDBytes)
	}
	if !s.Kind.valid() {
		return fmt.Errorf("hs: site %q has unknown kind %q", s.ID, s.Kind)
	}
	for _, f := range []struct {
		name, text string
		max        int
	}{{"msg", s.Msg, maxMsgBytes}, {"file", s.File, maxFileBytes}, {"func", s.Func, maxFuncBytes}} {
		if err := checkText(f.text, f.max); err != nil {
			return fmt.Errorf("hs: site %q %s: %w", s.ID, f.name, err)
		}
	}
	return nil
}

// checkText applies the protocol's free-text rule: valid UTF-8, within a byte limit, no control characters and
// no Unicode format or separator characters (Cf, Zl, Zp).
func checkText(text string, max int) error {
	if len(text) > max {
		return fmt.Errorf("%d bytes, limit is %d", len(text), max)
	}
	if !utf8.ValidString(text) {
		return errors.New("not valid UTF-8")
	}
	for _, r := range text {
		if unicode.IsControl(r) || unicode.In(r, unicode.Cf, unicode.Zl, unicode.Zp) {
			return fmt.Errorf("forbidden character U+%04X", r)
		}
	}
	return nil
}
