import fcntl
import os
import signal
import stat
import tempfile
import threading
import time
import unittest

from hairspring._output import HIGH_FD_SECOND, FdSink, open_sink


def read_all(fd: int) -> bytes:
    chunks = []
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            return b"".join(chunks)
        chunks.append(chunk)


class OpenTests(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.r, self.w = os.pipe()  # stands in for descriptor 2
        self.addCleanup(self._close_pipe)
        self.sinks: list[FdSink] = []

    def _close_pipe(self) -> None:
        for fd in (self.r, self.w):
            try:
                os.close(fd)
            except OSError:
                pass
        for sink in self.sinks:
            sink.close()

    def open(self, environ: dict[str, str], privileged: bool = False) -> tuple[FdSink | None, int]:
        sink, dropped = open_sink(environ, privileged, stderr_fd=self.w)
        if sink is not None:
            self.sinks.append(sink)
        return sink, dropped

    def drained(self) -> bytes:
        os.close(self.w)
        data = read_all(self.r)
        self.w = os.open(os.devnull, os.O_WRONLY)  # keeps _close_pipe simple
        return data

    def test_by_default_events_go_to_a_private_copy_of_standard_error(self) -> None:
        sink, dropped = self.open({})
        self.assertIsNotNone(sink)
        self.assertEqual(dropped, 0)
        # 200, or 20 when the descriptor limit is too low for 200.
        self.assertGreaterEqual(sink.fd, HIGH_FD_SECOND)  # type: ignore[union-attr]
        self.assertNotEqual(sink.fd, self.w)  # type: ignore[union-attr]
        self.assertTrue(sink.write_line(b"HAIRSPRING_EVENT {}\n"))  # type: ignore[union-attr]
        sink.close()  # type: ignore[union-attr]
        self.assertEqual(self.drained(), b"HAIRSPRING_EVENT {}\n")

    def test_the_private_copy_is_close_on_exec(self) -> None:
        sink, _ = self.open({})
        flags = fcntl.fcntl(sink.fd, fcntl.F_GETFD)  # type: ignore[union-attr]
        self.assertTrue(flags & fcntl.FD_CLOEXEC)

    def test_off_writes_nothing_and_opens_nothing(self) -> None:
        sink, dropped = self.open({"HAIRSPRING_EVENTS": "off"})
        self.assertIsNone(sink)
        self.assertEqual(dropped, 0)

    def test_a_closed_standard_error_means_no_sink(self) -> None:
        sink, dropped = open_sink({}, False, stderr_fd=4000)
        self.assertIsNone(sink)
        self.assertEqual(dropped, 0)

    def test_an_events_file_receives_the_lines_in_append_mode(self) -> None:
        path = os.path.join(self.dir.name, "events")
        with open(path, "wb") as f:
            f.write(b"existing\n")
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": path})
        self.assertEqual(dropped, 0)
        self.assertTrue(sink.write_line(b"one\n"))  # type: ignore[union-attr]
        sink.close()  # type: ignore[union-attr]
        with open(path, "rb") as f:
            self.assertEqual(f.read(), b"existing\none\n")

    def test_a_new_events_file_is_created_with_mode_0644(self) -> None:
        path = os.path.join(self.dir.name, "created")
        old = os.umask(0)
        try:
            sink, _ = self.open({"HAIRSPRING_EVENTS_FILE": path})
        finally:
            os.umask(old)
        sink.close()  # type: ignore[union-attr]
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o644)

    def test_an_events_file_that_cannot_be_opened_falls_back_and_is_counted(self) -> None:
        missing = os.path.join(self.dir.name, "no-such-dir", "events")
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": missing})
        self.assertEqual(dropped, 1)
        self.assertTrue(sink.write_line(b"to stderr\n"))  # type: ignore[union-attr]
        sink.close()  # type: ignore[union-attr]
        self.assertEqual(self.drained(), b"to stderr\n")

    def test_a_symlink_in_the_last_component_is_refused(self) -> None:
        target = os.path.join(self.dir.name, "target")
        link = os.path.join(self.dir.name, "link")
        with open(target, "wb"):
            pass
        os.symlink(target, link)
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": link})
        self.assertEqual(dropped, 1)
        sink.write_line(b"x\n")  # type: ignore[union-attr]
        sink.close()  # type: ignore[union-attr]
        self.assertEqual(os.path.getsize(target), 0)

    def test_a_fifo_without_a_reader_does_not_hang_and_is_refused(self) -> None:
        fifo = os.path.join(self.dir.name, "fifo")
        os.mkfifo(fifo)
        started = time.monotonic()
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": fifo})
        self.assertLess(time.monotonic() - started, 5)
        self.assertEqual(dropped, 1)
        self.assertIsNotNone(sink)

    def test_a_fifo_with_a_reader_is_refused_after_opening(self) -> None:
        fifo = os.path.join(self.dir.name, "fifo")
        os.mkfifo(fifo)
        reader = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
        self.addCleanup(os.close, reader)
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": fifo})
        self.assertEqual(dropped, 1)
        sink.write_line(b"x\n")  # type: ignore[union-attr]
        try:
            received = os.read(reader, 10)  # a FIFO whose writer has gone reads as end of file
        except BlockingIOError:
            received = b""
        self.assertEqual(received, b"", "nothing may be written into the FIFO")

    def test_a_device_is_refused(self) -> None:
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": os.devnull})
        self.assertEqual(dropped, 1)
        self.assertIsNotNone(sink)

    def test_a_privileged_process_ignores_the_events_file_setting(self) -> None:
        path = os.path.join(self.dir.name, "events")
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": path}, privileged=True)
        self.assertEqual(dropped, 0)
        self.assertFalse(os.path.exists(path))
        self.assertIsNotNone(sink)

    def test_an_empty_events_file_setting_is_ignored(self) -> None:
        sink, dropped = self.open({"HAIRSPRING_EVENTS_FILE": ""})
        self.assertEqual(dropped, 0)
        self.assertIsNotNone(sink)


class WriteTests(unittest.TestCase):
    def setUp(self) -> None:
        self.r, self.w = os.pipe()
        self.addCleanup(self._close)

    def _close(self) -> None:
        for fd in (self.r, self.w):
            try:
                os.close(fd)
            except OSError:
                pass

    def test_a_full_non_blocking_pipe_is_waited_for_not_truncated(self) -> None:
        os.set_blocking(self.w, False)
        filler = b"f" * 4096
        filled = 0
        while True:
            try:
                filled += os.write(self.w, filler)
            except BlockingIOError:
                break
        sink = FdSink(os.dup(self.w))
        self.addCleanup(sink.close)
        line = b"HAIRSPRING_EVENT " + b"x" * 3000 + b"\n"
        results: list[bool] = []
        worker = threading.Thread(target=lambda: results.append(sink.write_line(line)))
        worker.start()
        time.sleep(0.2)
        drained = b""
        os.set_blocking(self.r, False)
        deadline = time.monotonic() + 10
        while worker.is_alive() and time.monotonic() < deadline:
            try:
                drained += os.read(self.r, 65536)
            except BlockingIOError:
                time.sleep(0.01)
        worker.join(5)
        try:
            while True:
                chunk = os.read(self.r, 65536)
                if not chunk:
                    break
                drained += chunk
        except BlockingIOError:
            pass
        self.assertEqual(results, [True])
        self.assertTrue(drained.endswith(line))

    def test_a_reader_that_has_gone_is_a_failed_write_not_an_exception(self) -> None:
        r2, w2 = os.pipe()
        os.close(r2)
        broken = FdSink(w2)
        self.addCleanup(broken.close)
        self.assertFalse(broken.write_line(b"HAIRSPRING_EVENT {}\n"))

    def test_sigpipe_is_blocked_during_a_guarded_write_and_consumed_afterwards(self) -> None:
        r2, w2 = os.pipe()
        os.close(r2)
        broken = FdSink(w2)
        self.addCleanup(broken.close)
        previous = signal.signal(signal.SIGPIPE, signal.SIG_DFL)
        self.addCleanup(signal.signal, signal.SIGPIPE, previous)
        before = signal.pthread_sigmask(signal.SIG_BLOCK, set())
        with broken.guarded():
            self.assertIn(signal.SIGPIPE, signal.pthread_sigmask(signal.SIG_BLOCK, set()))
            self.assertFalse(broken.write_line(b"HAIRSPRING_EVENT {}\n"))
        self.assertEqual(signal.pthread_sigmask(signal.SIG_BLOCK, set()), before)
        self.assertNotIn(signal.SIGPIPE, signal.sigpending())

    def test_a_sigpipe_the_caller_had_blocked_is_left_pending(self) -> None:
        r2, w2 = os.pipe()
        os.close(r2)
        broken = FdSink(w2)
        self.addCleanup(broken.close)
        previous = signal.signal(signal.SIGPIPE, signal.SIG_DFL)
        self.addCleanup(signal.signal, signal.SIGPIPE, previous)
        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGPIPE})
        self.addCleanup(signal.pthread_sigmask, signal.SIG_UNBLOCK, {signal.SIGPIPE})
        with broken.guarded():
            broken.write_line(b"x\n")
        self.assertIn(signal.SIGPIPE, signal.pthread_sigmask(signal.SIG_BLOCK, set()))
        # Consume it here so unblocking in cleanup does not kill the test process.
        if signal.SIGPIPE in signal.sigpending():
            signal.sigwait({signal.SIGPIPE})


class VerifyTests(unittest.TestCase):
    def test_a_descriptor_that_now_names_another_file_is_given_up(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a = os.open(os.path.join(tmp, "a"), os.O_WRONLY | os.O_CREAT)
            b = os.open(os.path.join(tmp, "b"), os.O_WRONLY | os.O_CREAT)
            sink = FdSink(a)
            self.assertFalse(sink.verify())
            os.dup2(b, a)  # the program takes the descriptor away
            self.assertTrue(sink.verify())
            self.assertFalse(sink.verify(), "given up for good: reported lost once")
            self.assertTrue(sink.write_line(b"x\n"), "nothing is written, and it is not a new failure")
            os.close(b)
            sink.close()
            self.assertEqual(os.path.getsize(os.path.join(tmp, "a")), 0)
            self.assertEqual(os.path.getsize(os.path.join(tmp, "b")), 0)


if __name__ == "__main__":
    unittest.main()
