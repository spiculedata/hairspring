package hairspring

import (
	"encoding/binary"
	"io"
	"os"
	"runtime"
	"strconv"
)

// atSecure is the auxiliary-vector entry type the kernel sets when the program was started with elevated rights: a
// set-uid or set-gid binary, but also file capabilities or a security-module transition, which change no id.
const (
	atNull   = 0
	atSecure = 23
)

// isPrivileged reports whether the environment may not be trusted for a path to open. On Linux it asks the kernel
// (AT_SECURE in /proc/self/auxv); elsewhere it compares real and effective ids.
func isPrivileged() bool { return isPrivilegedWith(openAuxv, idsDiffer, runtime.GOOS == "linux") }

func openAuxv() (io.ReadCloser, error) { return os.Open("/proc/self/auxv") }

func idsDiffer() bool { return os.Getuid() != os.Geteuid() || os.Getgid() != os.Getegid() }

// isPrivilegedWith decides from the auxiliary vector when it can. An ordinary Linux process can always read its own
// vector; a file-capability binary that is not dumpable may not be able to, and that is the case AT_SECURE exists for,
// so on Linux a vector that cannot be read means privileged. A vector that reads but has no AT_SECURE entry (an old
// kernel) and every other platform fall back to comparing ids.
func isPrivilegedWith(open func() (io.ReadCloser, error), differ func() bool, linux bool) bool {
	secure, found, err := atSecureFrom(open)
	switch {
	case err != nil && linux:
		return true
	case found:
		return secure
	default:
		return differ()
	}
}

// atSecureFrom reads the auxiliary vector: pairs of machine words (type, value), ended by type 0. err is set when the
// vector could not be opened or read.
func atSecureFrom(open func() (io.ReadCloser, error)) (secure, found bool, err error) {
	r, err := open()
	if err != nil {
		return false, false, err
	}
	defer r.Close()
	raw, err := io.ReadAll(r)
	if err != nil {
		return false, false, err
	}
	word := strconv.IntSize / 8
	for off := 0; off+2*word <= len(raw); off += 2 * word {
		kind, value := readWord(raw[off:], word), readWord(raw[off+word:], word)
		if kind == atNull {
			break
		}
		if kind == atSecure {
			return value != 0, true, nil
		}
	}
	return false, false, nil
}

func readWord(b []byte, size int) uint64 {
	if size == 8 {
		return binary.NativeEndian.Uint64(b)
	}
	return uint64(binary.NativeEndian.Uint32(b))
}
