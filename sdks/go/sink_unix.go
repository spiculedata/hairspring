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

// privateCopy duplicates fd onto a descriptor above 2. dup returns the lowest free number, so when the program has
// closed fd 0, 1 or 2 the copy would land there: on fd 1 or 2 it would get the SIGPIPE behaviour it is meant to
// avoid, and fd 0 is the program's input. The low numbers dup hands out are held until one above 2 comes back, then
// released.
func privateCopy(fd int) (*os.File, error) {
	var held []int
	defer func() {
		for _, h := range held {
			_ = syscall.Close(h)
		}
	}()
	for {
		copied, err := syscall.Dup(fd)
		if err != nil {
			return nil, err
		}
		if copied > 2 {
			syscall.CloseOnExec(copied)
			return os.NewFile(uintptr(copied), "hairspring-report"), nil
		}
		held = append(held, copied)
	}
}
