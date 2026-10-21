"""Where the lines go: a private copy of stderr (or the events file) at a high descriptor number, so a program that
closes or reuses descriptor 2 cannot redirect the report. One `write(2)` per line in the normal case.

The behaviours here are the ones the Go and Rust SDKs settled on after review: the events file is opened without
following a symlink and without blocking, and must be a regular file; a process started with elevated rights ignores
the setting; a reader that has gone costs the report, not the process (SIGPIPE is blocked around the writes)."""

from __future__ import annotations

import contextlib
import errno
import fcntl
import os
import signal
import select
import stat
from collections.abc import Iterator, Mapping
from typing import Protocol

# The private events descriptor lives well above the ones a program numbers itself.
HIGH_FD_FIRST = 200
# Used when RLIMIT_NOFILE is too low for the first.
HIGH_FD_SECOND = 20
STDERR_FD = 2
# EINTR and EAGAIN retries per line; each EAGAIN waits up to WAIT_STEP_SECONDS, so a line gives up after about ten
# seconds without the reader taking a byte.
RETRY_LIMIT = 1000
WAIT_STEP_SECONDS = 0.01
FILE_MODE = 0o644
EVENTS_FILE_FLAGS = os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK


class Sink(Protocol):
    """Where finished lines go. `FdSink` is the real one; the tests use a list."""

    def write_line(self, data: bytes) -> bool: ...

    def verify(self) -> bool: ...

    def close(self) -> None: ...

    def guarded(self) -> contextlib.AbstractContextManager[None]: ...


class FdSink:
    """Writes whole lines to one descriptor it owns. It remembers which file the descriptor referred to at the start,
    so `verify` can tell if the program has since taken the descriptor away."""

    def __init__(self, fd: int) -> None:
        self.fd = fd
        info = os.fstat(fd)
        self._identity = (info.st_dev, info.st_ino)
        self._saw_epipe = False

    def close(self) -> None:
        fd, self.fd = self.fd, -1
        if fd >= 0:
            with contextlib.suppress(OSError):
                os.close(fd)

    def verify(self) -> bool:
        """Has the descriptor been lost to the program (a `dup2` onto it)? Once it has, it is given up for good, so no
        later line can follow it either. Returns true if it was lost by this call."""
        if self.fd < 0:
            return False
        try:
            info = os.fstat(self.fd)
            same = (info.st_dev, info.st_ino) == self._identity
        except OSError:
            same = False
        if same:
            return False
        self.fd = -1
        return True

    def write_line(self, data: bytes) -> bool:
        """Write all of `data`. A short write is completed; a line that still fails after part of it went out is closed
        with a newline so the next line does not glue onto the fragment. False means the line was not written."""
        fd = self.fd
        if fd < 0:
            return True  # given up deliberately: not a new failure
        done, retries = 0, 0
        while done < len(data):
            try:
                done += os.write(fd, data[done:])
            except InterruptedError:
                retries += 1
            except BlockingIOError:
                retries += 1
                select.select([], [fd], [], WAIT_STEP_SECONDS)  # a full non-blocking pipe: let the reader catch up
            except OSError as err:
                self._saw_epipe = self._saw_epipe or err.errno == errno.EPIPE
                return self._fail(fd, done)
            if retries > RETRY_LIMIT:
                return self._fail(fd, done)
        return True

    @staticmethod
    def _fail(fd: int, written: int) -> bool:
        if written:
            with contextlib.suppress(OSError):
                os.write(fd, b"\n")
        return False

    @contextlib.contextmanager
    def guarded(self) -> Iterator[None]:
        """Block SIGPIPE while lines are written, so a reader that has gone away costs the report, not the process.
        If a write raised a SIGPIPE and the caller did not already have it blocked, it is consumed before the mask is
        restored, so it is not delivered on unblocking."""
        pipe_only = {signal.SIGPIPE}
        previous = signal.pthread_sigmask(signal.SIG_BLOCK, pipe_only)
        self._saw_epipe = False
        try:
            yield
        finally:
            if self._saw_epipe and signal.SIGPIPE not in previous and signal.SIGPIPE in signal.sigpending():
                signal.sigwait(pipe_only)
            signal.pthread_sigmask(signal.SIG_SETMASK, previous)


def _private_copy(fd: int) -> tuple[int | None, bool]:
    """A copy of `fd` at a high number, close-on-exec. `(None, False)` if `fd` is not open; the flag says no high
    number was available, so the original stays exposed to the program (the SDK counts that as dropped)."""
    for first in (HIGH_FD_FIRST, HIGH_FD_SECOND):
        try:
            return fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, first), False
        except OSError:
            continue
    try:
        fcntl.fcntl(fd, fcntl.F_GETFD)
    except OSError:
        return None, False
    return fd, True


def _open_events_file(path: str) -> int:
    """Open for appending. The file must be a regular file: it is opened non-blocking, so a FIFO with no reader fails
    at once instead of hanging the program, and a FIFO or device that does open is refused."""
    fd = os.open(path, EVENTS_FILE_FLAGS, FILE_MODE)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise OSError(errno.EINVAL, "the events file is not a regular file", path)
    return fd


def _sink_for(fd: int) -> tuple[FdSink | None, int]:
    """A sink on a private copy of `fd` (which it takes over when no copy is possible)."""
    copy, exposed = _private_copy(fd)
    if copy is None:
        return None, 0
    return FdSink(copy), int(exposed)


def open_sink(
    environ: Mapping[str, str], privileged: bool, stderr_fd: int = STDERR_FD
) -> tuple[FdSink | None, int]:
    """Open the events channel from the environment. Returns the sink (None when output is off or stderr is not open)
    and how many losses to count as dropped (an events file that could not be opened, no room for a private copy)."""
    if environ.get("HAIRSPRING_EVENTS") == "off":
        return None, 0
    dropped = 0
    path = None if privileged else environ.get("HAIRSPRING_EVENTS_FILE")
    if path:
        try:
            fd = _open_events_file(path)
        except OSError:
            dropped += 1  # the file could not be opened: events fall back to stderr, and this is counted
        else:
            sink, exposed = _sink_for(fd)
            if sink is not None and sink.fd != fd:
                os.close(fd)
            return sink, dropped + exposed
    sink, exposed = _sink_for(stderr_fd)
    return sink, dropped + exposed
