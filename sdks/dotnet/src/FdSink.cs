namespace Hairspring;

/// <summary>Writes each line to a descriptor with <c>write(2)</c>: one call for a line this size. A reader that has gone costs the line, not the process.</summary>
internal sealed class FdSink : ISink
{
    private readonly int _fd;
    private readonly bool _ownsFd;

    public FdSink(int fd, bool ownsFd)
    {
        _fd = fd;
        _ownsFd = ownsFd;
    }

    public bool WriteLine(byte[] line) => Native.WriteAll(_fd, line);

    public void Close()
    {
        if (_ownsFd)
        {
            Native.Close(_fd);
        }
    }
}
