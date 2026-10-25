using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;

namespace Hairspring;

/// <summary>The checks that keep the events file a regular file nobody swapped: each one is made on its own, with the other checks out of the way.</summary>
internal static class EventsFileTests
{
    private static string Scratch()
    {
        var dir = Path.Combine(Path.GetTempPath(), "hs-dotnet-events-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

    private static EventsFile.Seams With(Func<string, StatResult>? stat = null, Func<string, int>? open = null, Action? afterLook = null) =>
        new(stat, open, afterLook);

    public static void Register(Suite s)
    {
        s.Add("events file: a path that is not a regular file is refused before anything is opened", () =>
        {
            var dir = Scratch();
            try
            {
                var opened = 0;
                var seams = With(open: p => { opened++; return Native.OpenAppend(p); });
                Check.Raises<IOException>(() => EventsFile.Open(dir, seams));
                Check.Equal(0, opened);
                if (File.Exists("/dev/null"))
                {
                    Check.Raises<IOException>(() => EventsFile.Open("/dev/null", seams));
                    Check.Equal(0, opened);
                }
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a descriptor that turns out not to be a regular file is closed and refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                File.WriteAllText(file, "x");
                var (read, write) = Posix.Pipe();
                Check.Raises<IOException>(() => EventsFile.Open(file, With(open: _ => write)));
                Check.No(Posix.IsOpen(write), "the refused descriptor was left open");
                Posix.Close(read);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a descriptor for another file than the one looked at is closed and refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var other = Path.Combine(dir, "other");
                File.WriteAllText(file, "x");
                File.WriteAllText(other, "y");
                var fd = -1;
                Check.Raises<IOException>(() => EventsFile.Open(file, With(open: _ => fd = Native.OpenAppend(other))));
                Check.Yes(fd >= 0, "the fake opened nothing");
                Check.No(Posix.IsOpen(fd), "the refused descriptor was left open");
                Check.Equal("y", File.ReadAllText(other));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a file swapped for another between the look and the open is refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var other = Path.Combine(dir, "other");
                File.WriteAllText(file, "x");
                File.WriteAllText(other, "y");
                Check.Raises<IOException>(() => EventsFile.Open(file, With(afterLook: () => File.Move(other, file, true))));
                Check.Equal("y", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a file that this call created and that was swapped before the open is refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var other = Path.Combine(dir, "other");
                File.WriteAllText(other, "y");
                Check.Raises<IOException>(() => EventsFile.Open(file, With(afterLook: () => File.Move(other, file, true))));
                Check.Equal("y", File.ReadAllText(file));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a hard link put in place of a file this call created is refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                var victim = Path.Combine(dir, "victim");
                File.WriteAllText(victim, "keep\n");
                Check.Raises<IOException>(() => EventsFile.Open(file, With(afterLook: () =>
                {
                    File.Delete(file);
                    Posix.HardLink(victim, file);
                })));
                Check.Equal("keep\n", File.ReadAllText(victim));
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: with no identity to compare against, a descriptor that is not a regular file is still refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                File.WriteAllText(file, "x"); // exists, so it cannot be created; the look is made to say it does not
                var (read, write) = Posix.Pipe();
                var seams = With(stat: _ => new StatResult(null, Native.NoSuchFile), open: _ => write);
                Check.Raises<IOException>(() => EventsFile.Open(file, seams));
                Check.No(Posix.IsOpen(write), "the refused descriptor was left open");
                Posix.Close(read);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("events file: a path that cannot be looked at for a reason other than not existing is refused", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "events");
                File.WriteAllText(file, "x");
                var opened = 0;
                Check.Raises<IOException>(() => EventsFile.Open(file, With(stat: _ => new StatResult(null, 5), open: p => { opened++; return Native.OpenAppend(p); })));
                Check.Equal(0, opened);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: the events file is opened without blocking, so a FIFO swapped in cannot hang the program", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "f");
                File.WriteAllText(file, "x");
                var fd = Native.OpenAppend(file);
                Check.Equal(Posix.NonBlock, Posix.Fcntl(fd, Posix.FdGetFl) & Posix.NonBlock);
                Posix.Close(fd);
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("native: the no-follow flag is known for the architectures listed and for no other", () =>
        {
            Check.Equal<int?>(0x20000, Native.NoFollowFor(Architecture.X64, true));
            Check.Equal<int?>(0x20000, Native.NoFollowFor(Architecture.X86, true));
            Check.Equal<int?>(0x8000, Native.NoFollowFor(Architecture.Arm64, true));
            Check.Equal<int?>(0x8000, Native.NoFollowFor(Architecture.Arm, true));
            Check.Equal<int?>(0x100, Native.NoFollowFor(Architecture.Arm64, false));
            foreach (var other in new[] { Architecture.RiscV64, Architecture.S390x, Architecture.LoongArch64, Architecture.Ppc64le })
            {
                Check.Equal<int?>(null, Native.NoFollowFor(other, true));
            }
        });
        s.Add("native: on an architecture whose flag values are not known the events file is refused and nothing is created", () =>
        {
            var dir = Scratch();
            try
            {
                var file = Path.Combine(dir, "f");
                Check.Yes(Native.OpenAppendWith(file, null) < 0, "refused");
                Check.No(File.Exists(file), "created");
            }
            finally
            {
                Directory.Delete(dir, true);
            }
        });
        s.Add("secure: a system that cannot say whether the process is set-uid counts as privileged", () =>
        {
            Check.Equal(1, Native.IsSetUgid(() => throw new EntryPointNotFoundException()));
            Check.Equal(0, Native.IsSetUgid(() => 0));
            Check.Equal(1, Native.IsSetUgid(() => 1));
        });
    }
}
