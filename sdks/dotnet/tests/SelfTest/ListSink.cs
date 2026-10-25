using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;

namespace Hairspring;

/// <summary>A sink that keeps the lines it is given, for tests. <c>refuse</c> makes it fail for the lines it matches.</summary>
internal sealed class ListSink : ISink
{
    private readonly List<string> _lines = new();
    private readonly Func<string, bool> _refuse;

    public ListSink() : this(_ => false)
    {
    }

    public ListSink(Func<string, bool> refuse)
    {
        _refuse = refuse;
    }

    public bool WriteLine(byte[] line)
    {
        var text = Encoding.ASCII.GetString(line);
        if (_refuse(text))
        {
            return false;
        }
        lock (_lines)
        {
            _lines.Add(text);
        }
        return true;
    }

    public void Close()
    {
    }

    public IReadOnlyList<string> Lines()
    {
        lock (_lines)
        {
            return _lines.ToArray();
        }
    }

    /// <summary>The lines of one kind, in the order written.</summary>
    public IReadOnlyList<string> OfKind(string kind)
    {
        var needle = $"\"kind\":\"{kind}\"";
        return Lines().Where(l => l.Contains(needle, StringComparison.Ordinal)).ToArray();
    }

    /// <summary>The kinds in the order written, one entry per line.</summary>
    public IReadOnlyList<string> Kinds() => Lines().Select(l =>
    {
        var start = l.IndexOf("\"kind\":\"", StringComparison.Ordinal) + 8;
        return l[start..l.IndexOf('"', start)];
    }).ToArray();
}
