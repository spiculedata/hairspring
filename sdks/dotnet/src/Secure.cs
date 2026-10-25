using System;
using System.Buffers.Binary;
using System.IO;
using System.Text;

namespace Hairspring;

/// <summary>
/// Was this process started with elevated rights? If so, <c>HAIRSPRING_EVENTS_FILE</c> is not trusted.
/// <para>On Linux the kernel is asked (<c>AT_SECURE</c> in <c>/proc/self/auxv</c>): a set-uid or set-gid binary sets it, and so
/// do file capabilities and a security-module transition, which change no id. A vector that cannot be read counts as
/// privileged. A vector that reads but has no <c>AT_SECURE</c> entry falls back to comparing real and effective ids in
/// <c>/proc/self/status</c>. On macOS and the BSDs <see cref="Native.IsSetUgid"/> (<c>issetugid</c>) is asked, and a failure to
/// ask counts as privileged.</para>
/// </summary>
internal static class Secure
{
    private const ulong AtNull = 0;
    private const ulong AtSecure = 23;
    private const string Auxv = "/proc/self/auxv";
    private const string Status = "/proc/self/status";
    private const int Word32 = 4;
    private const int Word64 = 8;

    private static ulong Word(ReadOnlySpan<byte> raw, bool littleEndian, int size) => size == Word32
        ? (littleEndian ? BinaryPrimitives.ReadUInt32LittleEndian(raw) : BinaryPrimitives.ReadUInt32BigEndian(raw))
        : (littleEndian ? BinaryPrimitives.ReadUInt64LittleEndian(raw) : BinaryPrimitives.ReadUInt64BigEndian(raw));

    /// <summary>The auxiliary vector: pairs of machine words (type, value), ended by type 0. Null when there is no AT_SECURE entry.</summary>
    public static bool? AtSecureFrom(byte[] raw, bool littleEndian, int wordSize)
    {
        var offset = 0;
        while (raw.Length - offset >= 2 * wordSize)
        {
            var kind = Word(raw.AsSpan(offset), littleEndian, wordSize);
            var value = Word(raw.AsSpan(offset + wordSize), littleEndian, wordSize);
            offset += 2 * wordSize;
            if (kind == AtNull)
            {
                break;
            }
            if (kind == AtSecure)
            {
                return value != 0;
            }
        }
        return null;
    }

    private static (long Real, long Effective)? IdPair(string status, string name)
    {
        foreach (var line in status.Split('\n'))
        {
            if (!line.StartsWith(name + ":", StringComparison.Ordinal))
            {
                continue;
            }
            var fields = line[(name.Length + 1)..].Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (fields.Length < 2
                || !long.TryParse(fields[0], System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var real)
                || !long.TryParse(fields[1], System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var effective))
            {
                return null;
            }
            return (real, effective);
        }
        return null;
    }

    /// <summary>Whether the real and effective user or group id differ, from the text of <c>/proc/self/status</c>; null if unreadable.</summary>
    public static bool? IdsDifferIn(byte[] statusText)
    {
        var text = Encoding.ASCII.GetString(statusText);
        if (IdPair(text, "Uid") is not { } uid || IdPair(text, "Gid") is not { } gid)
        {
            return null;
        }
        return uid.Real != uid.Effective || gid.Real != gid.Effective;
    }

    /// <summary>The Linux decision from the two files (each reader may throw <see cref="IOException"/>); false where there is no such file to ask.</summary>
    public static bool IsPrivilegedWith(Func<byte[]> auxv, Func<byte[]> status, bool linux, bool littleEndian = true, int wordSize = Word64)
    {
        if (!linux)
        {
            return false;
        }
        try
        {
            return AtSecureFrom(auxv(), littleEndian, wordSize) ?? IdsDifferIn(status()) ?? true;
        }
        catch (Exception unreadable) when (unreadable is IOException or UnauthorizedAccessException)
        {
            return true; // the case AT_SECURE exists for: a vector the process may not read
        }
    }

    /// <summary>Whether this process started with elevated rights, asked of the kernel.</summary>
    public static bool IsPrivileged()
    {
        if (OperatingSystem.IsLinux())
        {
            return IsPrivilegedWith(
                () => File.ReadAllBytes(Auxv),
                () => File.ReadAllBytes(Status),
                true,
                BitConverter.IsLittleEndian,
                IntPtr.Size == 4 ? Word32 : Word64);
        }
        if (OperatingSystem.IsMacOS() || OperatingSystem.IsFreeBSD())
        {
            return Native.IsSetUgid() != 0;
        }
        return false;
    }
}
