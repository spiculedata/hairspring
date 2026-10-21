package hairspring

import (
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"strconv"
	"testing"
)

// auxv builds the bytes of an auxiliary vector in this machine's word size and byte order.
func auxv(pairs ...uint64) func() (io.ReadCloser, error) {
	var b bytes.Buffer
	for _, v := range pairs {
		if strconv.IntSize == 64 {
			_ = binary.Write(&b, binary.NativeEndian, v)
		} else {
			_ = binary.Write(&b, binary.NativeEndian, uint32(v))
		}
	}
	return func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader(b.Bytes())), nil }
}

func TestPrivilegedReadsAtSecureFromTheAuxiliaryVector(t *testing.T) {
	differ := func() bool { return false }

	// AT_SECURE is entry type 23; a file-capability or LSM transition sets it with no uid change.
	if !isPrivilegedWith(auxv(6, 4096, 23, 1, 0, 0), differ) {
		t.Error("AT_SECURE=1 was not treated as privileged")
	}
	if isPrivilegedWith(auxv(6, 4096, 23, 0, 0, 0), func() bool { return true }) {
		t.Error("AT_SECURE=0 was overridden by the uid comparison")
	}
}

func TestPrivilegedFallsBackToTheIDComparison(t *testing.T) {
	noAtSecure := auxv(6, 4096, 0, 0)
	failing := func() (io.ReadCloser, error) { return nil, errors.New("no /proc") }

	cases := []struct {
		name string
		open func() (io.ReadCloser, error)
		diff bool
	}{
		{"no AT_SECURE entry, ids differ", noAtSecure, true},
		{"no AT_SECURE entry, ids same", noAtSecure, false},
		{"auxv unreadable, ids differ", failing, true},
		{"auxv unreadable, ids same", failing, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := isPrivilegedWith(c.open, func() bool { return c.diff }); got != c.diff {
				t.Fatalf("isPrivilegedWith = %v, want %v", got, c.diff)
			}
		})
	}
}

func TestPrivilegedIgnoresATruncatedVector(t *testing.T) {
	truncated := func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader([]byte{1, 2, 3})), nil }

	if isPrivilegedWith(truncated, func() bool { return false }) {
		t.Fatal("a truncated auxv was treated as privileged")
	}
}
