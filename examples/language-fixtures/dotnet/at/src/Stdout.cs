using System;
using System.Runtime.InteropServices;

namespace LangAt;

/// <summary>
/// Writes the result line to descriptor 1 with one <c>write(2)</c>. The .NET console stream would write to a duplicate of
/// descriptor 1 and could split a line; this is one call on descriptor 1 itself, whose result tells whether the whole line
/// went out (a full device or a closed descriptor is a failure, not a lost line).
/// </summary>
internal static partial class Stdout
{
    private const int StdoutDescriptor = 1;
    private const int Interrupted = 4;

    [LibraryImport("libc", EntryPoint = "write", SetLastError = true)]
    private static partial nint SysWrite(int fd, ref byte buffer, nint count);

    /// <summary>True when the kernel took the whole line.</summary>
    public static bool Write(byte[] line)
    {
        try
        {
            while (true)
            {
                var written = SysWrite(StdoutDescriptor, ref line[0], line.Length);
                if (written < 0 && Marshal.GetLastPInvokeError() == Interrupted)
                {
                    continue;
                }
                return written == line.Length;
            }
        }
        catch (Exception e) when (e is DllNotFoundException or EntryPointNotFoundException)
        {
            return false;
        }
    }
}
