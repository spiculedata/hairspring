namespace Hairspring;

/// <summary>
/// The report's way to standard error. Lines go through descriptor 2 itself, as the program's own writes do, for as long as
/// descriptor 2 is the open file it was at start. At start the sink also makes a private duplicate of descriptor 2 (the same
/// open file, so it shares the offset and the append mode; close-on-exec), and writes to that duplicate only once descriptor
/// 2 has been closed, is no longer open for writing, or has been replaced by a different file. A file is never reopened by
/// name, so no access the descriptor did not have is ever taken.
/// <para>Descriptor 2 that is not open for writing at start (closed with <c>2&gt;&amp;-</c>, where the runtime's first open file
/// may have been given the number, or a file opened read-only) is never written to: lines fail and the program goes on.</para>
/// </summary>
internal sealed class StderrSink : ISink
{
    private readonly int _fd;
    private readonly bool _usable;
    private readonly FileId? _identity;
    private int _copy;

    private StderrSink(int fd, bool usable, FileId? identity, int copy)
    {
        _fd = fd;
        _usable = usable;
        _identity = identity;
        _copy = copy;
    }

    /// <summary>Whether the private duplicate exists and is close-on-exec (it is closed at once when it cannot be marked).</summary>
    public bool CopyIsCloseOnExec => _copy >= 0;

    public static StderrSink Open(int fd = 2)
    {
        var seen = Native.StatFd(fd);
        if ((seen.Stat is null && seen.Errno != Native.Unavailable) || !Native.IsOpenForWriting(fd))
        {
            return new StderrSink(fd, false, null, -1);
        }
        var copy = Native.Dup(fd);
        if (copy >= 0 && !Native.SetCloseOnExec(copy))
        {
            Native.Close(copy);
            copy = -1;
        }
        return new StderrSink(fd, true, seen.Stat?.Id, copy);
    }

    /// <summary>Descriptor 2 is still the file it was, and is open for writing. If the identity could not be read at start, being open for writing is all that can be checked.</summary>
    private bool DescriptorIsOriginal()
    {
        if (!Native.IsOpenForWriting(_fd))
        {
            return false;
        }
        return _identity is not { } original || Native.StatFd(_fd).Stat?.Id == original;
    }

    public bool WriteLine(byte[] line)
    {
        if (!_usable)
        {
            return false;
        }
        if (DescriptorIsOriginal())
        {
            return Native.WriteAll(_fd, line);
        }
        return _copy >= 0 && Native.WriteAll(_copy, line);
    }

    public void Close()
    {
        if (_copy >= 0)
        {
            Native.Close(_copy);
            _copy = -1;
        }
    }
}
