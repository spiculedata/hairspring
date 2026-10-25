using System;
using System.Buffers.Binary;
using System.Runtime.InteropServices;
using System.Text;

namespace Hairspring;

/// <summary>What a path or descriptor is.</summary>
internal enum FileType
{
    Other,
    Regular,
    Directory,
    Fifo,
    CharDevice,
    BlockDevice,
    Socket,
    Symlink,
}

/// <summary>Which file a descriptor or path is: device and inode.</summary>
internal readonly record struct FileId(ulong Dev, ulong Ino);

internal readonly record struct FileStat(FileType Type, FileId Id);

/// <summary>A stat that worked (<c>Stat</c>) or the errno it failed with; <see cref="Native.Unavailable"/> when the call is not there.</summary>
internal readonly record struct StatResult(FileStat? Stat, int Errno);

/// <summary>
/// The few system calls the SDK makes, through the C library (no package, no native code of its own). Everything here
/// is fixed-arity: variadic functions (<c>fcntl</c> with an argument, <c>open</c> with <c>O_CREAT</c>) are not called, because
/// an Apple arm64 caller passes variadic arguments differently. A call that cannot be made (no such function, no such
/// library) fails like any other error; nothing here throws for a missing system call.
/// <para>Linux and macOS only. Linux uses <c>statx</c>, whose layout is the same on every architecture; macOS uses
/// <c>fstat</c>, <c>lstat</c> and <c>stat</c>.</para>
/// </summary>
internal static partial class Native
{
    public const int NoSuchFile = 2;
    public const int BadDescriptor = 9;
    public const int Unavailable = -1;

    private const int Interrupted = 4;
    private const int StatBufferSize = 256;
    private const int WriteOnly = 1;
    private const int AccessMask = 3;
    private const int GetFlags = 3;
    private const int AtFdCwd = -100;
    private const int AtEmptyPath = 0x1000;
    private const int AtSymlinkNoFollow = 0x100;
    private const uint StatxBasic = 0x7ff;
    private const int StatxModeOffset = 28;
    private const int StatxInoOffset = 32;
    private const int StatxDevMajorOffset = 136;
    private const int StatxDevMinorOffset = 140;
    private const int BsdDevOffset = 0;
    private const int BsdModeOffset = 4;
    private const int BsdInoOffset = 8;
    private const int FormatMask = 0xF000;

    private static readonly bool OnLinux = OperatingSystem.IsLinux();
    private static readonly bool OnMac = OperatingSystem.IsMacOS();
    private static readonly bool OnX86 = RuntimeInformation.ProcessArchitecture is Architecture.X64 or Architecture.X86;

    /// <summary>O_APPEND on this system.</summary>
    public static readonly int AppendFlag = OnLinux ? 0x400 : 0x8;

    private static readonly int NonBlockFlag = OnLinux ? 0x800 : 0x4;
    private static readonly int NoCttyFlag = OnLinux ? 0x100 : 0x20000;
    private static readonly int CloExecFlag = OnLinux ? 0x80000 : 0x1000000;
    private static readonly int? NoFollowFlag = NoFollowFor(RuntimeInformation.ProcessArchitecture, OnLinux);
    private static readonly nuint CloseOnExecRequest = OnLinux ? 0x5451u : 0x20006601u;

    /// <summary>
    /// <c>O_NOFOLLOW</c> where the value is known: Linux on x86 and x86-64 (0x20000) and on arm and arm64 (0x8000), and macOS
    /// (0x100). The value differs between Linux architectures (on riscv64, s390x and loongarch64 it is not either of those), so an
    /// architecture not listed gets null and the events file is refused there rather than opened with a wrong flag.
    /// </summary>
    public static int? NoFollowFor(Architecture architecture, bool linux)
    {
        if (!linux)
        {
            return 0x100;
        }
        return architecture switch
        {
            Architecture.X64 or Architecture.X86 => 0x20000,
            Architecture.Arm64 or Architecture.Arm => 0x8000,
            _ => null,
        };
    }

    [LibraryImport("libc", EntryPoint = "write", SetLastError = true)]
    private static partial nint SysWrite(int fd, ref byte buffer, nint count);

    [LibraryImport("libc", EntryPoint = "dup", SetLastError = true)]
    private static partial int SysDup(int fd);

    [LibraryImport("libc", EntryPoint = "close", SetLastError = true)]
    private static partial int SysClose(int fd);

    [LibraryImport("libc", EntryPoint = "fcntl", SetLastError = true)]
    private static partial int SysFcntl(int fd, int command);

    [LibraryImport("libc", EntryPoint = "ioctl", SetLastError = true)]
    private static partial int SysIoctl(int fd, nuint request);

    [LibraryImport("libc", EntryPoint = "open", SetLastError = true)]
    private static partial int SysOpen(ref byte path, int flags);

    [LibraryImport("libc", EntryPoint = "getpid")]
    private static partial int SysGetPid();

    [LibraryImport("libc", EntryPoint = "issetugid")]
    private static partial int SysIsSetUgid();

    [LibraryImport("libc", EntryPoint = "statx", SetLastError = true)]
    private static partial int SysStatx(int dirFd, ref byte path, int flags, uint mask, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "fstat", SetLastError = true)]
    private static partial int SysFstat(int fd, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "fstat$INODE64", SetLastError = true)]
    private static partial int SysFstatInode64(int fd, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "lstat", SetLastError = true)]
    private static partial int SysLstat(ref byte path, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "lstat$INODE64", SetLastError = true)]
    private static partial int SysLstatInode64(ref byte path, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "stat", SetLastError = true)]
    private static partial int SysStat(ref byte path, ref byte buffer);

    [LibraryImport("libc", EntryPoint = "stat$INODE64", SetLastError = true)]
    private static partial int SysStatInode64(ref byte path, ref byte buffer);

    static Native()
    {
        if (OnLinux)
        {
            try
            {
                NativeLibrary.SetDllImportResolver(typeof(Native).Assembly, ResolveLibc);
            }
            catch (InvalidOperationException)
            {
                // A resolver is already set for this assembly: the default probing stays.
            }
        }
    }

    /// <summary>"libc" is the C library on glibc systems even where only <c>libc.so.6</c> exists (no development package); anything else is left to the default probing.</summary>
    private static IntPtr ResolveLibc(string name, System.Reflection.Assembly assembly, DllImportSearchPath? searchPath) =>
        name == "libc" && NativeLibrary.TryLoad("libc.so.6", out var handle) ? handle : IntPtr.Zero;

    private static bool IsMissing(Exception e) => e is DllNotFoundException or EntryPointNotFoundException;

    private static byte[] CString(string path) => Encoding.UTF8.GetBytes(path + "\0");

    // ---- processes -----------------------------------------------------------------------------------------------

    /// <summary>The pid asked of the kernel each time (<see cref="Environment.ProcessId"/> is cached, so a forked child would still see its parent's).</summary>
    public static long GetPid()
    {
        try
        {
            return SysGetPid();
        }
        catch (Exception e) when (IsMissing(e))
        {
            return Environment.ProcessId;
        }
    }

    /// <summary>Non-zero when the process was started set-uid or set-gid (macOS and the BSDs); a failure to ask counts as privileged.</summary>
    public static int IsSetUgid(Func<int>? ask = null)
    {
        try
        {
            return (ask ?? SysIsSetUgid)();
        }
        catch (Exception e) when (IsMissing(e))
        {
            return 1;
        }
    }

    // ---- descriptors ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Writes all of <paramref name="data"/> with <c>write(2)</c>: one call for a line that fits, more only if the kernel took
    /// part of it. A call interrupted by a signal is repeated. False on any error, including a reader that has gone away
    /// (the runtime ignores SIGPIPE, so the error is <c>EPIPE</c>, not a signal).
    /// </summary>
    public static bool WriteAll(int fd, ReadOnlySpan<byte> data)
    {
        try
        {
            return WriteLoop(data, (ReadOnlySpan<byte> rest, out int errno) =>
            {
                var written = SysWrite(fd, ref MemoryMarshal.GetReference(rest), rest.Length);
                errno = written < 0 ? Marshal.GetLastPInvokeError() : 0;
                return written;
            });
        }
        catch (Exception e) when (IsMissing(e))
        {
            return false;
        }
    }

    /// <summary>One attempt to write <paramref name="rest"/>: the byte count, or -1 with the errno.</summary>
    internal delegate nint WriteStep(ReadOnlySpan<byte> rest, out int errno);

    /// <summary>The loop of <see cref="WriteAll"/> over any single write: pieces until the whole is out, an interrupted call repeated, any other error or a write of nothing a failure.</summary>
    internal static bool WriteLoop(ReadOnlySpan<byte> data, WriteStep step)
    {
        var offset = 0;
        while (offset < data.Length)
        {
            var written = step(data[offset..], out var errno);
            if (written < 0)
            {
                if (errno == Interrupted)
                {
                    continue;
                }
                return false;
            }
            if (written == 0)
            {
                return false;
            }
            offset += (int)written;
        }
        return true;
    }

    /// <summary>A duplicate of the descriptor (the lowest free number), or -1.</summary>
    public static int Dup(int fd)
    {
        try
        {
            return SysDup(fd);
        }
        catch (Exception e) when (IsMissing(e))
        {
            return -1;
        }
    }

    public static void Close(int fd)
    {
        try
        {
            SysClose(fd);
        }
        catch (Exception e) when (IsMissing(e))
        {
            // Nothing to close.
        }
    }

    /// <summary>Marks the descriptor close-on-exec (<c>FIOCLEX</c>, which takes no argument and so needs no variadic call).</summary>
    public static bool SetCloseOnExec(int fd)
    {
        try
        {
            return SysIoctl(fd, CloseOnExecRequest) == 0;
        }
        catch (Exception e) when (IsMissing(e))
        {
            return false;
        }
    }

    /// <summary>Whether the descriptor is open for writing (its access mode is not read-only). False for a closed descriptor.</summary>
    public static bool IsOpenForWriting(int fd)
    {
        try
        {
            var flags = SysFcntl(fd, GetFlags);
            return flags >= 0 && (flags & AccessMask) != 0;
        }
        catch (Exception e) when (IsMissing(e))
        {
            return false;
        }
    }

    /// <summary>
    /// Opens a path write-only for appending, not following a link in the last component, not blocking, not becoming a
    /// controlling terminal, close-on-exec; never creating. The descriptor, or a negative errno.
    /// </summary>
    public static int OpenAppend(string path) => OpenAppendWith(path, NoFollowFlag);

    /// <summary>As <see cref="OpenAppend"/>, with the no-follow flag given; without one (null) nothing is opened.</summary>
    public static int OpenAppendWith(string path, int? noFollow)
    {
        if ((!OnLinux && !OnMac) || noFollow is not { } noFollowFlag)
        {
            return Unavailable;
        }
        try
        {
            var bytes = CString(path);
            var flags = WriteOnly | AppendFlag | noFollowFlag | NonBlockFlag | NoCttyFlag | CloExecFlag;
            var fd = SysOpen(ref bytes[0], flags);
            return fd >= 0 ? fd : -Marshal.GetLastPInvokeError();
        }
        catch (Exception e) when (IsMissing(e))
        {
            return Unavailable;
        }
    }

    // ---- what a file is ------------------------------------------------------------------------------------------

    private static FileType TypeOf(uint mode) => (mode & FormatMask) switch
    {
        0x8000 => FileType.Regular,
        0x4000 => FileType.Directory,
        0x1000 => FileType.Fifo,
        0x2000 => FileType.CharDevice,
        0x6000 => FileType.BlockDevice,
        0xC000 => FileType.Socket,
        0xA000 => FileType.Symlink,
        _ => FileType.Other,
    };

    private static StatResult FromStatx(ReadOnlySpan<byte> buffer)
    {
        var mode = MemoryMarshal.Read<ushort>(buffer[StatxModeOffset..]);
        var ino = MemoryMarshal.Read<ulong>(buffer[StatxInoOffset..]);
        var major = MemoryMarshal.Read<uint>(buffer[StatxDevMajorOffset..]);
        var minor = MemoryMarshal.Read<uint>(buffer[StatxDevMinorOffset..]);
        return new StatResult(new FileStat(TypeOf(mode), new FileId(((ulong)major << 32) | minor, ino)), 0);
    }

    private static StatResult FromBsdStat(ReadOnlySpan<byte> buffer)
    {
        var dev = MemoryMarshal.Read<uint>(buffer[BsdDevOffset..]);
        var mode = MemoryMarshal.Read<ushort>(buffer[BsdModeOffset..]);
        var ino = MemoryMarshal.Read<ulong>(buffer[BsdInoOffset..]);
        return new StatResult(new FileStat(TypeOf(mode), new FileId(dev, ino)), 0);
    }

    private static StatResult Failed(int result) =>
        new(null, result == 0 ? Unavailable : Marshal.GetLastPInvokeError());

    /// <summary>What an open descriptor is. A closed one is <see cref="BadDescriptor"/>.</summary>
    public static StatResult StatFd(int fd)
    {
        if (!OnLinux && !OnMac)
        {
            return new StatResult(null, Unavailable);
        }
        try
        {
            var buffer = new byte[StatBufferSize];
            if (OnLinux)
            {
                var empty = new byte[1];
                var rc = SysStatx(fd, ref empty[0], AtEmptyPath, StatxBasic, ref buffer[0]);
                return rc == 0 ? FromStatx(buffer) : Failed(rc);
            }
            var result = OnX86 ? SysFstatInode64(fd, ref buffer[0]) : SysFstat(fd, ref buffer[0]);
            return result == 0 ? FromBsdStat(buffer) : Failed(result);
        }
        catch (Exception e) when (IsMissing(e))
        {
            return new StatResult(null, Unavailable);
        }
    }

    /// <summary>What a path is, looking at a link itself (<paramref name="followLinks"/> false) or at what it points to.</summary>
    public static StatResult StatPath(string path, bool followLinks)
    {
        if (!OnLinux && !OnMac)
        {
            return new StatResult(null, Unavailable);
        }
        try
        {
            var buffer = new byte[StatBufferSize];
            var bytes = CString(path);
            if (OnLinux)
            {
                var rc = SysStatx(AtFdCwd, ref bytes[0], followLinks ? 0 : AtSymlinkNoFollow, StatxBasic, ref buffer[0]);
                return rc == 0 ? FromStatx(buffer) : Failed(rc);
            }
            var result = (followLinks, OnX86) switch
            {
                (true, true) => SysStatInode64(ref bytes[0], ref buffer[0]),
                (true, false) => SysStat(ref bytes[0], ref buffer[0]),
                (false, true) => SysLstatInode64(ref bytes[0], ref buffer[0]),
                (false, false) => SysLstat(ref bytes[0], ref buffer[0]),
            };
            return result == 0 ? FromBsdStat(buffer) : Failed(result);
        }
        catch (Exception e) when (IsMissing(e))
        {
            return new StatResult(null, Unavailable);
        }
    }
}
