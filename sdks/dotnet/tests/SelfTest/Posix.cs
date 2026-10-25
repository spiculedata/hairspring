using System;
using System.IO;
using System.Runtime.InteropServices;

namespace Hairspring;

/// <summary>System calls the tests make on purpose (the SDK has its own in <see cref="Native"/>): pipes, duplicating and closing descriptors.</summary>
internal static partial class Posix
{
    public const int OpenReadOnly = 0;
    public const int OpenWriteOnly = 1;
    public const int OpenReadWrite = 2;
    public static readonly int NonBlock = OperatingSystem.IsLinux() ? 0x800 : 0x4;
    public const int FdGetFd = 1;
    public const int FdGetFl = 3;
    public const int CloseOnExec = 1;

    [LibraryImport("libc", EntryPoint = "pipe", SetLastError = true)]
    private static partial int SysPipe(ref int fds);

    [LibraryImport("libc", EntryPoint = "close", SetLastError = true)]
    public static partial int Close(int fd);

    [LibraryImport("libc", EntryPoint = "dup2", SetLastError = true)]
    public static partial int Dup2(int from, int to);

    [LibraryImport("libc", EntryPoint = "fcntl", SetLastError = true)]
    public static partial int Fcntl(int fd, int command);

    [LibraryImport("libc", EntryPoint = "open", SetLastError = true)]
    private static partial int SysOpen(ref byte path, int flags);

    [LibraryImport("libc", EntryPoint = "read", SetLastError = true)]
    private static partial nint SysRead(int fd, ref byte buffer, nint count);

    [LibraryImport("libc", EntryPoint = "write", SetLastError = true)]
    private static partial nint SysWrite(int fd, ref byte buffer, nint count);

    /// <summary>A pipe: the read end and the write end.</summary>
    public static (int Read, int Write) Pipe()
    {
        var fds = new int[2];
        if (SysPipe(ref fds[0]) != 0)
        {
            throw new IOException("pipe failed: " + Marshal.GetLastPInvokeError());
        }
        return (fds[0], fds[1]);
    }

    /// <summary>Opens an existing file with the given access flags; throws on failure.</summary>
    public static int Open(string path, int flags)
    {
        var bytes = System.Text.Encoding.UTF8.GetBytes(path + "\0");
        var fd = SysOpen(ref bytes[0], flags);
        return fd >= 0 ? fd : throw new IOException($"open {path} failed: {Marshal.GetLastPInvokeError()}");
    }

    public static void Write(int fd, string text)
    {
        var bytes = System.Text.Encoding.ASCII.GetBytes(text);
        if (SysWrite(fd, ref bytes[0], bytes.Length) != bytes.Length)
        {
            throw new IOException("write failed: " + Marshal.GetLastPInvokeError());
        }
    }

    /// <summary>Reads what is in a pipe or file now (up to 64 KiB) without waiting for more than the first read.</summary>
    public static string ReadSome(int fd)
    {
        var buffer = new byte[65536];
        var count = SysRead(fd, ref buffer[0], buffer.Length);
        return count <= 0 ? "" : System.Text.Encoding.ASCII.GetString(buffer, 0, (int)count);
    }

    public static bool IsOpen(int fd) => Fcntl(fd, FdGetFd) >= 0;
}
