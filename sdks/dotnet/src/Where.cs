using System;

namespace Hairspring;

/// <summary>The source location of a call site, from the compiler (<c>CallerFilePath</c>, <c>CallerLineNumber</c>): free at run time, no stack walk.</summary>
internal static class Where
{
    public const string UnknownFile = "<unknown>";

    /// <summary>The last two parts of a path (<c>src/Race.cs</c>), so a build machine's directories are not sent.</summary>
    public static string Short(string? path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return UnknownFile;
        }
        var parts = path.Split(['/', '\\'], StringSplitOptions.RemoveEmptyEntries);
        return parts.Length switch
        {
            0 => UnknownFile,
            1 => parts[0],
            _ => parts[^2] + "/" + parts[^1],
        };
    }
}
