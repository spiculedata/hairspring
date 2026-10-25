using System;
using System.Collections.Generic;

namespace Hairspring;

/// <summary>The lines written from a <see cref="Snapshot"/>, without any lock held.</summary>
internal static class Report
{
    /// <summary>Sends lines and counts the ones that were lost. A line that does not fit, or cannot be written, is counted, never truncated.</summary>
    private sealed class Writer
    {
        private readonly ISink _sink;
        private readonly Action _noteDrop;

        public Writer(ISink sink, Action noteDrop)
        {
            _sink = sink;
            _noteDrop = noteDrop;
        }

        public ulong Lost { get; private set; }

        public void Send(Func<byte[]> build)
        {
            bool written;
            try
            {
                written = _sink.WriteLine(build());
            }
            catch (Exception)
            {
                written = false;
            }
            if (!written)
            {
                Lost++;
                _noteDrop();
            }
        }
    }

    /// <summary>
    /// Writes hello, one catalog line per (id, kind), one assert per id, work, a thread line per thread, then end.
    /// <c>snapshot.Dropped</c> is the loss count before the report; <c>end</c> adds the lines lost during it.
    /// <paramref name="noteDrop"/> is called for every lost line.
    /// </summary>
    public static void Write(Snapshot snapshot, Identity identity, ISink sink, EndReason reason, long status, Action noteDrop)
    {
        var info = new Rows.Proc(identity.Tag, identity.TagStable, false, snapshot.CatalogHash, snapshot.NIds, snapshot.Dropped);
        var writer = new Writer(sink, noteDrop);
        writer.Send(() => Lines.Hello(info));
        foreach (var row in snapshot.Catalog)
        {
            writer.Send(() => Lines.Catalog(info, row));
        }
        foreach (var row in snapshot.Asserts)
        {
            writer.Send(() => Lines.AssertLine(info, row));
        }
        writer.Send(() => Lines.Work(info, snapshot.Work));
        foreach (var view in snapshot.Threads)
        {
            writer.Send(() => Lines.Thread(info, new Rows.ThreadRow(view.Key, view.Evals, view.TapeConsumed, view.Stable)));
        }
        var closing = info with { Dropped = Saturating.Add(info.Dropped, writer.Lost) };
        writer.Send(() => Lines.End(closing, reason, status, snapshot.Evals, snapshot.Threads.Count));
    }
}
