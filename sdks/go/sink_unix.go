//go:build unix

package hairspring

import (
	"os"
	"syscall"
)

// openAppend opens path for appending, creating it if needed. O_APPEND makes each write land at the end of the file
// even when several processes share it, and O_NOFOLLOW refuses a symbolic link.
func openAppend(path string) (*os.File, error) {
	return os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND|syscall.O_NOFOLLOW, 0o644)
}
