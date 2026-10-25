using System.Runtime.InteropServices;

namespace HsSample;

/// <summary>System calls a sample makes on purpose: closing descriptor 2, forking, ending the process without running exit handlers.</summary>
internal static partial class Posix
{
    [LibraryImport("libc", EntryPoint = "close")]
    public static partial int Close(int fd);

    [LibraryImport("libc", EntryPoint = "fork")]
    public static partial int Fork();

    [LibraryImport("libc", EntryPoint = "_exit")]
    public static partial void Halt(int status);

    [LibraryImport("libc", EntryPoint = "waitpid")]
    private static partial int WaitPid(int pid, ref int status, int options);

    public static void WaitFor(int pid)
    {
        var status = 0;
        WaitPid(pid, ref status, 0);
    }
}
