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

type brokenReader struct{}

func (brokenReader) Read([]byte) (int, error) { return 0, errors.New("read failed") }
func (brokenReader) Close() error             { return nil }

const (
	onLinux   = true
	elsewhere = false
)

func TestPrivilegedReadsAtSecureFromTheAuxiliaryVector(t *testing.T) {
	differ := func() bool { return false }

	// AT_SECURE is entry type 23; a file-capability or LSM transition sets it with no uid change.
	if !isPrivilegedWith(auxv(6, 4096, 23, 1, 0, 0), differ, onLinux) {
		t.Error("AT_SECURE=1 was not treated as privileged")
	}
	if isPrivilegedWith(auxv(6, 4096, 23, 0, 0, 0), func() bool { return true }, onLinux) {
		t.Error("AT_SECURE=0 was overridden by the uid comparison")
	}
}

func TestPrivilegedFallsBackToTheIDComparisonWhenTheVectorHasNoAtSecure(t *testing.T) {
	noAtSecure := auxv(6, 4096, 0, 0)

	for _, diff := range []bool{true, false} {
		for _, linux := range []bool{onLinux, elsewhere} {
			if got := isPrivilegedWith(noAtSecure, func() bool { return diff }, linux); got != diff {
				t.Errorf("no AT_SECURE entry, ids differ=%v, linux=%v: got %v", diff, linux, got)
			}
		}
	}
}

// An ordinary process can always read its own auxv. A file-capability binary that is not dumpable may not, and that
// is exactly the case AT_SECURE exists for, so on Linux an unreadable vector means privileged.
func TestPrivilegedTreatsAnUnreadableVectorAsPrivilegedOnLinux(t *testing.T) {
	cannotOpen := func() (io.ReadCloser, error) { return nil, errors.New("permission denied") }
	cannotRead := func() (io.ReadCloser, error) { return brokenReader{}, nil }
	idsSame := func() bool { return false }

	for name, open := range map[string]func() (io.ReadCloser, error){"open fails": cannotOpen, "read fails": cannotRead} {
		if !isPrivilegedWith(open, idsSame, onLinux) {
			t.Errorf("%s: an unreadable auxv on Linux was not treated as privileged", name)
		}
		if isPrivilegedWith(open, idsSame, elsewhere) {
			t.Errorf("%s: off Linux there is no auxv to read, so the id comparison decides", name)
		}
		if !isPrivilegedWith(open, func() bool { return true }, elsewhere) {
			t.Errorf("%s: off Linux, differing ids must still mean privileged", name)
		}
	}
}

func TestPrivilegedIgnoresATruncatedVector(t *testing.T) {
	truncated := func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader([]byte{1, 2, 3})), nil }

	if isPrivilegedWith(truncated, func() bool { return false }, onLinux) {
		t.Fatal("a truncated auxv was treated as privileged")
	}
}
