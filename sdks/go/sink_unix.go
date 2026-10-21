//go:build unix

package hairspring

import (
	"fmt"
	"io"
	"os"
	"syscall"
)

// openAppend opens path for appending, creating it if needed, and requires a regular file. O_APPEND makes each write
// land at the end of the file even when several processes share it. O_NOFOLLOW refuses a symbolic link in the last
// component of the path only; a link in a directory component is followed. The open is non-blocking so that a FIFO
// with no reader fails at once instead of hanging the program; the descriptor is made blocking again afterwards.
func openAppend(path string) (*os.File, error) {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND|syscall.O_NOFOLLOW|syscall.O_NONBLOCK, 0o644)
	if err != nil {
		return nil, err
	}
	info, err := f.Stat()
	if err != nil {
		_ = f.Close()
		return nil, err
	}
	if !info.Mode().IsRegular() {
		_ = f.Close()
		return nil, fmt.Errorf("hairspring: %s is not a regular file", path)
	}
	if err := syscall.SetNonblock(int(f.Fd()), false); err != nil {
		_ = f.Close()
		return nil, err
	}
	return f, nil
}

// privateStderr returns a private copy of file descriptor 2. Go ends a program that writes to a closed pipe on fd 1
// or 2 with SIGPIPE, but only returns EPIPE for any other descriptor, so a reader that went away costs the report and
// not the program's exit status. The copy also survives the program closing or reusing fd 2.
func privateStderr() io.Writer {
	if f, err := privateCopy(int(os.Stderr.Fd())); err == nil {
		return f
	}
	return os.Stderr
}

func privateCopy(fd int) (*os.File, error) {
	copied, err := syscall.Dup(fd)
	if err != nil {
		return nil, err
	}
	if copied <= 2 { // fd 1 or 2 had been closed: the copy would get the SIGPIPE behaviour it is meant to avoid
		_ = syscall.Close(copied)
		return nil, fmt.Errorf("hairspring: no descriptor above 2 for the report")
	}
	syscall.CloseOnExec(copied)
	return os.NewFile(uintptr(copied), "hairspring-report"), nil
}
