package hairspring

import (
	"encoding/binary"
	"io"
	"os"
	"strconv"
)

// atSecure is the auxiliary-vector entry type the kernel sets when the program was started with elevated rights: a
// set-uid or set-gid binary, but also file capabilities or a security-module transition, which change no id.
const (
	atNull   = 0
	atSecure = 23
)

// isPrivileged reports whether the environment may not be trusted for a path to open. On Linux it asks the kernel
// (AT_SECURE in /proc/self/auxv); elsewhere, or when that is unreadable, it compares real and effective ids.
func isPrivileged() bool { return isPrivilegedWith(openAuxv, idsDiffer) }

func openAuxv() (io.ReadCloser, error) { return os.Open("/proc/self/auxv") }

func idsDiffer() bool { return os.Getuid() != os.Geteuid() || os.Getgid() != os.Getegid() }

func isPrivilegedWith(open func() (io.ReadCloser, error), differ func() bool) bool {
	if secure, found := atSecureFrom(open); found {
		return secure
	}
	return differ()
}

// atSecureFrom reads the auxiliary vector: pairs of machine words (type, value), ended by type 0.
func atSecureFrom(open func() (io.ReadCloser, error)) (secure, found bool) {
	r, err := open()
	if err != nil {
		return false, false
	}
	defer r.Close()
	raw, err := io.ReadAll(r)
	if err != nil {
		return false, false
	}
	word := strconv.IntSize / 8
	for off := 0; off+2*word <= len(raw); off += 2 * word {
		kind, value := readWord(raw[off:], word), readWord(raw[off+word:], word)
		if kind == atNull {
			return false, false
		}
		if kind == atSecure {
			return value != 0, true
		}
	}
	return false, false
}

func readWord(b []byte, size int) uint64 {
	if size == 8 {
		return binary.NativeEndian.Uint64(b)
	}
	return uint64(binary.NativeEndian.Uint32(b))
}
