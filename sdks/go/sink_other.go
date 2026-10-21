//go:build !unix

package hairspring

import (
	"io"
	"os"
)

// openAppend opens path for appending, creating it if needed. This platform has no O_NOFOLLOW, so a symbolic link is
// followed, and a non-regular file is not refused; Hairspring's backends run on Linux, where sink_unix.go applies.
func openAppend(path string) (*os.File, error) {
	return os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o644)
}

// privateStderr is standard error itself on this platform.
func privateStderr() io.Writer { return os.Stderr }
