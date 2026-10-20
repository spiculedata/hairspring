package hs

import (
	"strings"
	"testing"
)

func TestSiteValidateAcceptsOrdinarySite(t *testing.T) {
	// Arrange
	s := Site{ID: "go.sql.scan_returns_own_row", Kind: KindAlways, Msg: "scan must return the caller's row", File: "sqlcancel.go", Line: 42, Func: "worker"}

	// Act
	err := s.Validate()

	// Assert
	if err != nil {
		t.Fatalf("Validate() = %v, want nil", err)
	}
}

func TestSiteValidateRejectsBadSites(t *testing.T) {
	cases := map[string]Site{
		"empty id":            {ID: "", Kind: KindAlways},
		"id with space":       {ID: "a b", Kind: KindAlways},
		"id over 128 bytes":   {ID: strings.Repeat("a", 129), Kind: KindAlways},
		"id with non-ascii":   {ID: "café", Kind: KindAlways},
		"unknown kind":        {ID: "x", Kind: Kind("sometimes_maybe")},
		"empty kind":          {ID: "x"},
		"msg over 512 bytes":  {ID: "x", Kind: KindAlways, Msg: strings.Repeat("m", 513)},
		"msg with newline":    {ID: "x", Kind: KindAlways, Msg: "a\nb"},
		"msg with zero width": {ID: "x", Kind: KindAlways, Msg: "a​b"},
		"msg with line sep":   {ID: "x", Kind: KindAlways, Msg: "a b"},
		"msg invalid utf8":    {ID: "x", Kind: KindAlways, Msg: "a\xffb"},
		"file with control":   {ID: "x", Kind: KindAlways, File: "a\tb.go"},
		"func over 128 bytes": {ID: "x", Kind: KindAlways, Func: strings.Repeat("f", 129)},
	}
	for name, s := range cases {
		t.Run(name, func(t *testing.T) {
			if err := s.Validate(); err == nil {
				t.Fatalf("Validate(%+v) = nil, want an error", s)
			}
		})
	}
}

func TestKindProperties(t *testing.T) {
	cases := []struct {
		kind    Kind
		mustHit bool
		isPoint bool
	}{
		{KindAlways, true, false},
		{KindAlwaysOrUnreachable, false, false},
		{KindSometimes, true, false},
		{KindReachable, true, true},
		{KindUnreachable, false, true},
	}
	for _, c := range cases {
		if got := c.kind.mustHit(); got != c.mustHit {
			t.Errorf("%s.mustHit() = %v, want %v", c.kind, got, c.mustHit)
		}
		if got := c.kind.isPoint(); got != c.isPoint {
			t.Errorf("%s.isPoint() = %v, want %v", c.kind, got, c.isPoint)
		}
		if !c.kind.valid() {
			t.Errorf("%s.valid() = false", c.kind)
		}
	}
}
