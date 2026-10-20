package hairspring

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

// The assertion kinds of protocol v1 (see the table in docs/guest-protocol.md).
const (
	// KindAlways must be true at every evaluation, and must be evaluated at least once per run.
	KindAlways Kind = "always"
	// KindAlwaysOrUnreachable must be true at every evaluation; never evaluating it is fine.
	KindAlwaysOrUnreachable Kind = "always_or_unreachable"
	// KindSometimes must be true at least once per run; a false evaluation is not a failure.
	KindSometimes Kind = "sometimes"
	// KindReachable must be reached at least once per run. It has no condition.
	KindReachable Kind = "reachable"
	// KindUnreachable must never be reached. It has no condition.
	KindUnreachable Kind = "unreachable"
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

// Protocol limits that the SDK enforces itself (bytes).
const (
	maxIDBytes   = 128
	maxMsgBytes  = 512
	maxFileBytes = 256
	maxFuncBytes = 128
)

var identPattern = regexp.MustCompile(`^[A-Za-z0-9._:/-]+$`)

// Site describes one assertion. Sites are plain values; the catalog never changes one after Declare.
type Site struct {
	ID   string // 1 to 128 bytes of [A-Za-z0-9._:/-]; the same id at two sites is one property
	Kind Kind
	Msg  string // optional, at most 512 bytes
	File string // optional location, at most 256 bytes
	Line uint64
	Func string // optional, at most 128 bytes
}

// Validate reports why a site cannot be sent under protocol v1, or nil.
func (s Site) Validate() error {
	if len(s.ID) > maxIDBytes || !identPattern.MatchString(s.ID) {
		return fmt.Errorf("hairspring: invalid id %q (1-%d bytes of [A-Za-z0-9._:/-])", s.ID, maxIDBytes)
	}
	if !s.Kind.valid() {
		return fmt.Errorf("hairspring: site %q has unknown kind %q", s.ID, s.Kind)
	}
	for _, f := range []struct {
		name, text string
		max        int
	}{{"msg", s.Msg, maxMsgBytes}, {"file", s.File, maxFileBytes}, {"func", s.Func, maxFuncBytes}} {
		if err := checkText(f.text, f.max); err != nil {
			return fmt.Errorf("hairspring: site %q %s: %w", s.ID, f.name, err)
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
