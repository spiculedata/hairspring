using System;
using System.IO;

namespace Hairspring;

/// <summary>
/// Opens <c>HAIRSPRING_EVENTS_FILE</c>. The file must be a regular file and is never reached through a link in its last
/// component. The path is looked at without following links first (a FIFO, a device, a directory or a link is refused before
/// anything is opened for writing). A missing file is created exclusively (<c>O_EXCL</c>, which a link, even a dangling one,
/// defeats) and its identity is read through the handle that created it. The file is then opened for appending with <c>O_NOFOLLOW</c>, <c>O_NONBLOCK</c> (a FIFO swapped in at the last
/// moment cannot hang the program), <c>O_NOCTTY</c> and close-on-exec, and looked at again through the descriptor: one that is
/// not a regular file, or not the file first seen, is closed and refused.
/// </summary>
internal static class EventsFile
{
    private const UnixFileMode CreateMode = UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.GroupRead | UnixFileMode.OtherRead;

    /// <summary>Replacements for the system calls and a point to act between the steps, for the tests; the real program passes none.</summary>
    internal sealed record Seams(Func<string, StatResult>? Stat, Func<string, int>? Open, Action? AfterLook);

    /// <summary>
    /// Creates the file exclusively and returns the identity of the file this call created, read through the handle it created it
    /// with (so a file put in the path afterwards cannot be taken for it); null when it exists already or cannot be created. The
    /// open below decides in that case, and fails with its own error.
    /// </summary>
    private static FileId? CreateExclusively(string path)
    {
        try
        {
            var options = new FileStreamOptions
            {
                Mode = FileMode.CreateNew,
                Access = FileAccess.Write,
                Share = FileShare.ReadWrite,
                UnixCreateMode = CreateMode,
            };
            using var created = new FileStream(path, options);
            var seen = Native.StatFd((int)created.SafeFileHandle.DangerousGetHandle());
            return seen.Stat is { Type: FileType.Regular } regular ? regular.Id : null;
        }
        catch (IOException)
        {
            return null;
        }
    }

    public static ISink Open(string path, Seams? seams = null)
    {
        if (string.IsNullOrEmpty(path))
        {
            throw new IOException("the events file path is empty");
        }
        var before = (seams?.Stat ?? (p => Native.StatPath(p, false)))(path);
        FileId? known = null;
        if (before.Stat is { } seen)
        {
            if (seen.Type != FileType.Regular)
            {
                throw new IOException($"the events file is not a regular file: {path}");
            }
            known = seen.Id;
        }
        else if (before.Errno != Native.NoSuchFile)
        {
            throw new IOException($"the events file cannot be looked at (error {before.Errno}): {path}");
        }
        known ??= CreateExclusively(path);
        seams?.AfterLook?.Invoke();
        var fd = (seams?.Open ?? Native.OpenAppend)(path);
        if (fd < 0)
        {
            throw new IOException($"the events file cannot be opened (error {-fd}): {path}");
        }
        var after = Native.StatFd(fd);
        if (after.Stat is not { Type: FileType.Regular } regular || (known is { } id && id != regular.Id))
        {
            Native.Close(fd);
            throw new IOException($"the events file is not the regular file that was looked at: {path}");
        }
        return new FdSink(fd, true);
    }
}
