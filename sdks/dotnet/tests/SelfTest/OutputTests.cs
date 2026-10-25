using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Hairspring;

/// <summary>Where the lines go: standard error, or an events file that must be a regular file and is never followed through a link.</summary>
internal static class OutputTests
{
    private static readonly TimeSpan Blocked = TimeSpan.FromSeconds(10);

    private static byte[] Bytes(string text) => Encoding.ASCII.GetBytes(text);

    private static string Scratch()
    {
        var dir = Path.Combine(Path.GetTempPath(), "hs-dotnet-output-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

    /// <summary>Opens the events file on another thread: a FIFO opened for writing with no reader would block for ever.</summary>
    private static ISink OpenWithin(string path)
    {
        var opening = Task.Run(() => EventsFile.Open(path));
        try
        {
            if (!opening.Wait(Blocked))
            {
                throw new CheckFailedException("opening the events file blocked");
            }
        }
        catch (AggregateException failed)
        {
            System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(failed.InnerException!).Throw();
        }
        return opening.Result;
    }

    private static Func<string, string?> Env(params (string Name, string Value)[] entries) =>
        name => Array.Find(entries, e => e.Name == name) is { Name: not null } found ? found.Value : null;

    private static bool MakeFifo(string path)
    {
        try
        {
            using var made = Process.Start(new ProcessStartInfo("mkfifo", path) { RedirectStandardError = true })!;
            made.WaitForExit();
            return made.ExitCode == 0;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return false;
        }
    }

    private static void EventsFileCases(Suite s)
    {
        s.Add("output: the events file receives appended lines in order", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                File.WriteAllText(file, "before\n");
                var sink = OpenWithin(file);
                Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
                Check.Yes(sink.WriteLine(Bytes("b\n")), "b");
                sink.Close();
                Check.Equal("before\na\nb\n", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: two writers on one events file never split a line", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var one = OpenWithin(file);
                var two = OpenWithin(file);
                var line = new string('x', 200) + "\n";
                Parallel.Invoke(
                    () => { for (var i = 0; i < 200; i++) { one.WriteLine(Bytes(line)); } },
                    () => { for (var i = 0; i < 200; i++) { two.WriteLine(Bytes(line)); } });
                one.Close();
                two.Close();
                var lines = File.ReadAllLines(file);
                Check.Equal(400, lines.Length);
                Check.Yes(Array.TrueForAll(lines, l => l.Length == 200), "a line was split");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a new events file is writable by its owner only", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                OpenWithin(file).Close();
                var mode = File.GetUnixFileMode(file);
                Check.Yes((mode & UnixFileMode.UserWrite) != 0, "owner writes");
                Check.Yes((mode & (UnixFileMode.GroupWrite | UnixFileMode.OtherWrite)) == 0, "others write");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a symbolic link as the events file is refused and its target untouched", () =>
        {
            var dir = Scratch();
            try
            {
                var target = Path.Combine(dir, "target");
                File.WriteAllText(target, "x\n");
                var link = Path.Combine(dir, "link");
                File.CreateSymbolicLink(link, target);
                Check.Raises<IOException>(() => OpenWithin(link));
                Check.Equal("x\n", File.ReadAllText(target));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a dangling link is refused and creates nothing", () =>
        {
            var dir = Scratch();
            try
            {
                var link = Path.Combine(dir, "link");
                File.CreateSymbolicLink(link, Path.Combine(dir, "nowhere"));
                Check.Raises<IOException>(() => OpenWithin(link));
                Check.No(File.Exists(Path.Combine(dir, "nowhere")), "the link's target was created");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a directory as the events file is refused", () =>
        {
            var dir = Scratch();
            try
            {
                Check.Raises<IOException>(() => OpenWithin(dir));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a FIFO with no reader is refused without blocking", () =>
        {
            var dir = Scratch();
            try
            {
                var fifo = Path.Combine(dir, "fifo");
                if (!MakeFifo(fifo))
                {
                    Console.WriteLine("     (skipped: no mkfifo here)");
                    return;
                }
                Check.Raises<IOException>(() => OpenWithin(fifo));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a FIFO with a reader is refused too, and nothing is written to it", () =>
        {
            var dir = Scratch();
            try
            {
                var fifo = Path.Combine(dir, "fifo");
                if (!MakeFifo(fifo))
                {
                    Console.WriteLine("     (skipped: no mkfifo here)");
                    return;
                }
                var reader = Posix.Open(fifo, Posix.OpenReadOnly | Posix.NonBlock);
                Posix.Close(reader);
                Check.Raises<IOException>(() => OpenWithin(fifo));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: a device as the events file is refused", () =>
        {
            if (!File.Exists("/dev/null"))
            {
                Console.WriteLine("     (skipped: no /dev/null here)");
                return;
            }
            Check.Raises<IOException>(() => OpenWithin("/dev/null"));
        });
        s.Add("output: a missing directory is an error the caller counts", () =>
            Check.Raises<IOException>(() => OpenWithin("/nonexistent-dir-for-hairspring/events")));
        s.Add("output: an empty path is an error", () => Check.Raises<IOException>(() => OpenWithin("")));
    }

    private static void StderrCases(Suite s)
    {
        s.Add("stderr: while descriptor 2 is valid the line goes through it", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write);
            Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
            Check.Equal("a\n", Posix.ReadSome(read));
            sink.Close();
            Posix.Close(read);
            Posix.Close(write);
        });
        s.Add("stderr: once descriptor 2 is closed the line goes to the private copy", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write);
            Posix.Close(write);
            Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
            Check.Yes(sink.WriteLine(Bytes("b\n")), "b");
            Check.Equal("a\nb\n", Posix.ReadSome(read));
            sink.Close();
            Posix.Close(read);
        });
        s.Add("stderr: a different file on descriptor 2 does not receive the report", () =>
        {
            var dir = Scratch();
            try
            {
                var other = Path.Combine(dir, "other");
                File.WriteAllText(other, "keep\n");
                var (read, write) = Posix.Pipe();
                var sink = StderrSink.Open(write);
                var replacement = Posix.Open(other, Posix.OpenReadWrite);
                Posix.Dup2(replacement, write); // the program (or a library) replaced descriptor 2
                Posix.Close(replacement);
                Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
                Check.Equal("a\n", Posix.ReadSome(read));
                Check.Equal("keep\n", File.ReadAllText(other));
                sink.Close();
                Posix.Close(read);
                Posix.Close(write);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("stderr: a read-only file on descriptor 2 is never written to", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "ro");
                File.WriteAllText(file, "keep\n");
                var fd = Posix.Open(file, Posix.OpenReadOnly);
                var sink = StderrSink.Open(fd);
                Check.No(sink.WriteLine(Bytes("a\n")), "written");
                Check.Equal(-1, sink.CopyDescriptor); // a descriptor that is not open for writing is not duplicated
                sink.Close();
                Posix.Close(fd);
                Check.Equal("keep\n", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("stderr: a file system that cannot say what descriptor 2 is does not silence the report", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write, _ => new StatResult(null, 1)); // EPERM, as a filtered statx answers
            Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
            Check.Equal("a\n", Posix.ReadSome(read));
            sink.Close();
            Posix.Close(read);
            Posix.Close(write);
        });
        s.Add("stderr: a descriptor that was never open writes nothing and does not throw", () =>
        {
            var (read, write) = Posix.Pipe();
            Posix.Close(read);
            Posix.Close(write);
            var sink = StderrSink.Open(write);
            Check.No(sink.WriteLine(Bytes("a\n")), "written");
            sink.Close();
        });
        s.Add("stderr: a regular file behind descriptor 2 keeps the program's own writes and the report in order", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "err");
                File.WriteAllText(file, "");
                var fd = Posix.Open(file, Posix.OpenWriteOnly);
                Posix.Write(fd, "AAAA\n");
                var sink = StderrSink.Open(fd);
                Check.Yes(sink.WriteLine(Bytes("report\n")), "report");
                Posix.Write(fd, "BBBB\n"); // the program goes on writing: it must not overwrite the report
                sink.Close();
                Posix.Close(fd);
                Check.Equal("AAAA\nreport\nBBBB\n", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("stderr: while descriptor 2 is the file it was, the report goes through it even if there is no private copy", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write, null, _ => -1);
            Check.Equal(-1, sink.CopyDescriptor);
            Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
            Check.Equal("a\n", Posix.ReadSome(read));
            sink.Close();
            Posix.Close(read);
            Posix.Close(write);
        });
        s.Add("stderr: a reader that has gone costs the line, not the process", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write);
            Posix.Close(read);
            Check.No(sink.WriteLine(Bytes("a\n")), "written");
            Check.No(sink.WriteLine(Bytes("b\n")), "written");
            sink.Close();
            Posix.Close(write);
        });
        s.Add("stderr: a file reopened read-only on descriptor 2 does not take the report, the private copy does", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "err");
                File.WriteAllText(file, "");
                var fd = Posix.Open(file, Posix.OpenWriteOnly);
                var sink = StderrSink.Open(fd);
                var readOnly = Posix.Open(file, Posix.OpenReadOnly);
                Posix.Dup2(readOnly, fd); // the same file, but no longer open for writing
                Posix.Close(readOnly);
                Check.Yes(sink.WriteLine(Bytes("a\n")), "a");
                sink.Close();
                Posix.Close(fd);
                Check.Equal("a\n", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("stderr: the private copy does not outlive an exec", () =>
        {
            var (read, write) = Posix.Pipe();
            var sink = StderrSink.Open(write);
            Check.Yes(sink.CopyIsCloseOnExec, "close-on-exec");
            Check.Equal(Posix.CloseOnExec, Posix.Fcntl(sink.CopyDescriptor, Posix.FdGetFd) & Posix.CloseOnExec);
            sink.Close();
            Posix.Close(read);
            Posix.Close(write);
        });
    }

    private static void ChoiceCases(Suite s)
    {
        s.Add("output: events off writes nothing and costs nothing", () =>
        {
            var opened = Output.Open(Env(("HAIRSPRING_EVENTS", "off")), false, () => new ListSink());
            Check.Yes(opened.Sink is null, "no sink");
            Check.Equal(0UL, opened.Dropped);
        });
        s.Add("output: without a file the report goes to standard error", () =>
        {
            var err = new ListSink();
            var opened = Output.Open(Env(), false, () => err);
            opened.Sink!.WriteLine(Bytes("x\n"));
            Check.Equal(new[] { "x\n" }, err.Lines());
            Check.Equal(0UL, opened.Dropped);
        });
        s.Add("output: an events file that cannot be opened falls back to standard error and is counted", () =>
        {
            var err = new ListSink();
            var opened = Output.Open(Env(("HAIRSPRING_EVENTS_FILE", "/nonexistent-dir-for-hairspring/events")), false, () => err);
            opened.Sink!.WriteLine(Bytes("x\n"));
            Check.Equal(new[] { "x\n" }, err.Lines());
            Check.Equal(1UL, opened.Dropped);
        });
        s.Add("output: a privileged process ignores the events file setting", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var err = new ListSink();
                var opened = Output.Open(Env(("HAIRSPRING_EVENTS_FILE", file)), true, () => err);
                opened.Sink!.WriteLine(Bytes("x\n"));
                Check.Equal(new[] { "x\n" }, err.Lines());
                Check.No(File.Exists(file), "the file was created");
                Check.Equal(0UL, opened.Dropped);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: the events file is used when it can be opened", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var err = new ListSink();
                var opened = Output.Open(Env(("HAIRSPRING_EVENTS_FILE", file)), false, () => err);
                opened.Sink!.WriteLine(Bytes("x\n"));
                opened.Sink!.Close();
                Check.Equal("x\n", File.ReadAllText(file));
                Check.Equal(0, err.Lines().Count);
                Check.Equal(0UL, opened.Dropped);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("output: an empty events file setting is the same as none", () =>
        {
            var err = new ListSink();
            var opened = Output.Open(Env(("HAIRSPRING_EVENTS_FILE", "")), false, () => err);
            Check.Equal(0UL, opened.Dropped);
            Check.Yes(ReferenceEquals(opened.Sink, err), "standard error");
        });
    }

    public static void Register(Suite s)
    {
        EventsFileCases(s);
        StderrCases(s);
        ChoiceCases(s);
    }
}
