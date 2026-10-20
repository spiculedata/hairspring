//go:build !unix

package hairspring

import "os"

// openAppend opens path for appending, creating it if needed. This platform has no O_NOFOLLOW, so a symbolic link is
// followed; Hairspring's backends run on Linux, where sink_unix.go applies.
func openAppend(path string) (*os.File, error) {
	return os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o644)
}
