using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;

namespace Hairspring;

/// <summary>The system calls the SDK makes: what a descriptor or a path is, writing whole lines, and the options of an open.</summary>
internal static class NativeTests
{
    private static string Scratch()
    {
        var dir = Path.Combine(Path.GetTempPath(), "hs-dotnet-native-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

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

    public static void Register(Suite s)
    {
        s.Add("native: the pid is the process's own", () => Check.Equal((long)Environment.ProcessId, Native.GetPid()));
        s.Add("native: a regular file, a directory, a pipe and a device are told apart", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "f");
                File.WriteAllText(file, "x");
                Check.Equal(FileType.Regular, Native.StatPath(file, false).Stat!.Value.Type);
                Check.Equal(FileType.Directory, Native.StatPath(dir, false).Stat!.Value.Type);
                var (read, write) = Posix.Pipe();
                Check.Equal(FileType.Fifo, Native.StatFd(read).Stat!.Value.Type);
                Posix.Close(read);
                Posix.Close(write);
                if (File.Exists("/dev/null"))
                {
                    Check.Equal(FileType.CharDevice, Native.StatPath("/dev/null", true).Stat!.Value.Type);
                }
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: a named pipe is a fifo", () =>
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
                Check.Equal(FileType.Fifo, Native.StatPath(fifo, false).Stat!.Value.Type);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: a link is seen as a link without following it and as its target when followed", () =>
        {
            var dir = Scratch();
            try
            {
                var target = Path.Combine(dir, "target");
                File.WriteAllText(target, "x");
                var link = Path.Combine(dir, "link");
                File.CreateSymbolicLink(link, target);
                Check.Equal(FileType.Symlink, Native.StatPath(link, false).Stat!.Value.Type);
                Check.Equal(FileType.Regular, Native.StatPath(link, true).Stat!.Value.Type);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: two descriptors of one file have one identity, two files have two", () =>
        {
            var dir = Scratch();
            try
            {
                var a = Path.Combine(dir, "a");
                var b = Path.Combine(dir, "b");
                File.WriteAllText(a, "x");
                File.WriteAllText(b, "x");
                var one = Posix.Open(a, Posix.OpenReadOnly);
                var two = Posix.Open(a, Posix.OpenReadOnly);
                var other = Posix.Open(b, Posix.OpenReadOnly);
                Check.Equal(Native.StatFd(one).Stat!.Value.Id, Native.StatFd(two).Stat!.Value.Id);
                Check.No(Native.StatFd(one).Stat!.Value.Id == Native.StatFd(other).Stat!.Value.Id, "two files, one identity");
                foreach (var fd in new[] { one, two, other })
                {
                    Posix.Close(fd);
                }
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: a closed descriptor and a missing path are errors, not types", () =>
        {
            var (read, write) = Posix.Pipe();
            Posix.Close(read);
            Posix.Close(write);
            var closed = Native.StatFd(read);
            Check.Yes(closed.Stat is null && closed.Errno == Native.BadDescriptor, "closed fd: errno " + closed.Errno);
            var missing = Native.StatPath("/nonexistent-dir-for-hairspring/x", false);
            Check.Yes(missing.Stat is null && missing.Errno == Native.NoSuchFile, "missing path: errno " + missing.Errno);
        });
        s.Add("native: writing a line is one call of write on the descriptor", () =>
        {
            var (read, write) = Posix.Pipe();
            Check.Yes(Native.WriteAll(write, "one\n"u8), "written");
            Check.Equal("one\n", Posix.ReadSome(read));
            Posix.Close(read);
            Posix.Close(write);
        });
        s.Add("native: a write that the kernel takes in pieces is written in pieces until the whole line is out", () =>
        {
            var taken = new List<string>();
            var ok = Native.WriteLoop("abcdefgh"u8, (ReadOnlySpan<byte> rest, out int errno) =>
            {
                errno = 0;
                var n = Math.Min(3, rest.Length);
                taken.Add(System.Text.Encoding.ASCII.GetString(rest[..n]));
                return n;
            });
            Check.Yes(ok, "written");
            Check.Equal(new[] { "abc", "def", "gh" }, taken.ToArray());
        });
        s.Add("native: a write interrupted by a signal is repeated, and any other error costs the line", () =>
        {
            var calls = 0;
            Check.Yes(Native.WriteLoop("abc"u8, (ReadOnlySpan<byte> rest, out int errno) =>
            {
                calls++;
                errno = calls == 1 ? 4 : 0; // EINTR once
                return calls == 1 ? -1 : rest.Length;
            }), "written after the interrupt");
            Check.Equal(2, calls);
            Check.No(Native.WriteLoop("abc"u8, (ReadOnlySpan<byte> rest, out int errno) =>
            {
                errno = 32; // EPIPE
                return -1;
            }), "written");
            Check.No(Native.WriteLoop("abc"u8, (ReadOnlySpan<byte> rest, out int errno) =>
            {
                errno = 0;
                return 0;
            }), "a write of nothing is not progress");
        });
        s.Add("native: writing to a pipe nobody reads costs the line, not the process", () =>
        {
            var (read, write) = Posix.Pipe();
            Posix.Close(read);
            Check.No(Native.WriteAll(write, "lost\n"u8), "written");
            Posix.Close(write);
        });
        s.Add("native: writing to a closed descriptor fails", () =>
        {
            var (read, write) = Posix.Pipe();
            Posix.Close(read);
            Posix.Close(write);
            Check.No(Native.WriteAll(write, "x\n"u8), "written");
        });
        s.Add("native: a duplicate is close-on-exec and shares the open file", () =>
        {
            var (read, write) = Posix.Pipe();
            var copy = Native.Dup(write);
            Check.Yes(copy >= 0, "dup");
            Check.Yes(Native.SetCloseOnExec(copy), "close-on-exec");
            Check.Equal(Posix.CloseOnExec, Posix.Fcntl(copy, Posix.FdGetFd) & Posix.CloseOnExec);
            Check.Yes(Native.WriteAll(copy, "via copy\n"u8), "written");
            Check.Equal("via copy\n", Posix.ReadSome(read));
            Posix.Close(copy);
            Posix.Close(read);
            Posix.Close(write);
        });
        s.Add("native: writability is the access mode of the open file", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "f");
                File.WriteAllText(file, "x");
                var readOnly = Posix.Open(file, Posix.OpenReadOnly);
                var readWrite = Posix.Open(file, Posix.OpenReadWrite);
                Check.No(Native.IsOpenForWriting(readOnly), "read-only");
                Check.Yes(Native.IsOpenForWriting(readWrite), "read-write");
                Posix.Close(readOnly);
                Posix.Close(readWrite);
                Check.No(Native.IsOpenForWriting(readOnly), "closed");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: the events file is opened for appending without following links", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "f");
                File.WriteAllText(file, "x");
                var fd = Native.OpenAppend(file);
                Check.Yes(fd >= 0, "opened");
                Check.Equal(FileType.Regular, Native.StatFd(fd).Stat!.Value.Type);
                Check.Equal(Native.AppendFlag, Posix.Fcntl(fd, Posix.FdGetFl) & Native.AppendFlag);
                Check.Equal(Posix.CloseOnExec, Posix.Fcntl(fd, Posix.FdGetFd) & Posix.CloseOnExec);
                Posix.Close(fd);
                var link = Path.Combine(dir, "link");
                File.CreateSymbolicLink(link, file);
                Check.Yes(Native.OpenAppend(link) < 0, "a link is refused");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
    }
}
