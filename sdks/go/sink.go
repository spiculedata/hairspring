package hairspring

import "io"

// eventSink is where report lines go. Each write is one write call carrying one whole line.
type eventSink struct {
	write   func([]byte) error
	close   func() error
	dropped uint64 // 1 when the events file was asked for and could not be opened
}

// eventsFilePath returns the file named by HAIRSPRING_EVENTS_FILE, or "" when the report should go to standard
// error: the variable is unset or empty, events are off, or the process is set-uid or set-gid (a path taken from the
// environment must not be opened with elevated rights).
func eventsFilePath(getenv func(string) string, privileged bool) string {
	if privileged || getenv("HAIRSPRING_EVENTS") == "off" {
		return ""
	}
	return getenv("HAIRSPRING_EVENTS_FILE")
}

// openSink opens the events file for appending, or uses stderr when path is "". A file that cannot be opened (a
// missing directory, a symbolic link, no permission) is not fatal: the report falls back to stderr and the failure is
// counted in dropped, which the end line carries.
func openSink(path string, stderr io.Writer) eventSink {
	toStderr := eventSink{
		write: func(l []byte) error { _, err := stderr.Write(l); return err },
		close: func() error { return nil },
	}
	if path == "" {
		return toStderr
	}
	f, err := openAppend(path)
	if err != nil {
		toStderr.dropped = 1
		return toStderr
	}
	return eventSink{
		write: func(l []byte) error { _, err := f.Write(l); return err },
		close: f.Close,
	}
}
