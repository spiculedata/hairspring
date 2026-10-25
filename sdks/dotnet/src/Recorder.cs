using System;
using System.Collections.Generic;
using System.Threading;

namespace Hairspring;

/// <summary>
/// The process-wide state: sites, thread slots, the lock that guards them, and the report.
/// <para>Every change to a counter happens with one monitor held, so counts are exact under any number of threads and a
/// snapshot taken under the same monitor shows every count, witness and thread total at one moment. The critical sections
/// are a few field updates, and nothing outside the SDK is called inside them: the report's lines are written (and the
/// sink called) with no lock held. The fast path of a call is one concurrent-map read to find the site and one
/// <see cref="ThreadLocal{T}"/> read to find the thread's slot.</para>
/// </summary>
internal sealed class Recorder
{
    /// <summary>How long the exit hook waits for a report another thread is writing.</summary>
    private static readonly TimeSpan WriterWait = TimeSpan.FromSeconds(5);

    private const long SigAbort = 6;

    private readonly Identity _identity;
    private readonly ISink? _sink;
    private readonly long _ownerPid;
    private readonly Func<long> _currentPid;
    private readonly object _lock = new();
    private readonly ThreadLocal<Slot?> _local = new();
    private readonly SiteTable _sites = new();
    private readonly SlotTable _slots = new();
    private ulong _dropped;
    private long? _status;
    private bool _reported;
    private bool _writing;

    public Recorder(Identity identity, ISink? sink, ulong dropped, long ownerPid, Func<long>? currentPid = null)
    {
        _identity = identity;
        _sink = sink;
        _dropped = dropped;
        _ownerPid = ownerPid;
        _currentPid = currentPid ?? Native.GetPid;
    }

    // ---- starting up ---------------------------------------------------------------------------------------------

    /// <summary>The recorder for a process: the tag comes from the environment; <paramref name="openingLosses"/> are counted as dropped.</summary>
    public static Recorder FromEnvironment(Func<string, string?> getenv, ISink? sink, ulong openingLosses, long pid, Func<long>? currentPid = null)
    {
        var identity = Identity.Resolve(getenv(Identity.EnvTag), getenv(Identity.EnvPid), pid);
        return new Recorder(identity, sink, openingLosses + (identity.Refused ? 1UL : 0UL), pid, currentPid);
    }

    /// <summary>
    /// The recorder for this process: tag and output from the environment, the report registered for the end of the process.
    /// Anything that goes wrong while starting (an unsupported system, a missing C library) gives a recorder that counts and
    /// writes nothing: the SDK never stops a program from starting.
    /// </summary>
    public static Recorder Start()
    {
        try
        {
            Func<string, string?> getenv = Environment.GetEnvironmentVariable;
            var opened = Output.Open(getenv, Secure.IsPrivileged(), () => StderrSink.Open());
            var recorder = FromEnvironment(getenv, opened.Sink, opened.Dropped, Native.GetPid());
            recorder.InstallExitHooks();
            return recorder;
        }
        catch (Exception)
        {
            return new Recorder(new Identity(Identity.DefaultTag, true, false), null, 0, 0);
        }
    }

    private void InstallExitHooks()
    {
        AppDomain.CurrentDomain.ProcessExit += (_, _) => ReportAtExit();
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            if (e.IsTerminating)
            {
                ReportAtCrash();
            }
        };
    }

    /// <summary>
    /// Says on the report's own output that the report failed. Not a protocol line, so a parser ignores it; the missing end line is
    /// what the engine sees. It goes through the sink, so it follows the same rules as the report (the events file, or descriptor 2 only
    /// while it is still the file it was) and says nothing when the output is off.
    /// </summary>
    public void WarnAboutFailure(string what, Exception failed)
    {
        var text = System.Text.Encoding.ASCII.GetBytes($"hairspring: {what}: {failed.GetType().Name}: {Fmt.JsonText(failed.Message, Fmt.MsgMax).Trim('"')}\n");
        try
        {
            _sink?.WriteLine(text);
        }
        catch (Exception)
        {
            // Nothing is left to tell.
        }
    }

    private void ReportAtExit()
    {
        try
        {
            Report(EndReason.Exit, null);
        }
        catch (Exception failed)
        {
            WarnAboutFailure("the report could not be written", failed);
        }
    }

    /// <summary>An exception nobody handled ends the process with SIGABRT, and no exit event follows: report now, as a signal.</summary>
    private void ReportAtCrash()
    {
        try
        {
            Report(EndReason.Signal, SigAbort);
        }
        catch (Exception failed)
        {
            WarnAboutFailure("the report could not be written", failed);
        }
    }

    // ---- state ---------------------------------------------------------------------------------------------------

    public void SetStatus(long value)
    {
        lock (_lock)
        {
            _status = value;
        }
    }

    private void NoteDrop()
    {
        lock (_lock)
        {
            _dropped = Saturating.Add(_dropped, 1);
        }
    }

    private Slot ClaimSlot()
    {
        Slot slot;
        lock (_lock)
        {
            var claim = _slots.Claim();
            slot = claim.Slot;
            if (claim.Shared)
            {
                _dropped = Saturating.Add(_dropped, 1); // a thread that shares the overflow slot
            }
        }
        _local.Value = slot;
        return slot;
    }

    private Slot CurrentSlot() => _local.Value ?? ClaimSlot();

    /// <summary>The site for an id and kind, registered when it is new; null when the table is full. A known site still checks the message: it is not part of the lookup key.</summary>
    private Site? SiteFor(Kind kind, string id, string msg, string file, long line)
    {
        ArgumentNullException.ThrowIfNull(id);
        var site = _sites.Find(kind, id);
        if (site is not null)
        {
            Args.CheckMsg(msg);
            return ReferenceEquals(site, Site.Untracked) ? null : site;
        }
        var shortFile = Where.Short(file); // only for a new site, never on the hot path
        lock (_lock)
        {
            return _sites.Register(kind, id, msg, shortFile, line);
        }
    }

    // ---- the call sites ------------------------------------------------------------------------------------------

    /// <summary>Put a site in the catalog without evaluating it, so a site that is never reached still shows up.</summary>
    public void Declare(Kind kind, string id, string msg, string file, long line)
    {
        KindInfo.Check(kind);
        if (SiteFor(kind, id, msg, file, line) is null)
        {
            NoteDrop();
        }
    }

    /// <summary>
    /// Count one evaluation. The first evaluation on each side of a site keeps its witness. A point kind
    /// (<c>Reachable</c>, <c>Unreachable</c>) has no condition: reaching it counts as true.
    /// </summary>
    public void Record(Kind kind, bool ok, string id, string msg, ReadOnlySpan<long> vals, string file, long line)
    {
        KindInfo.Check(kind);
        Args.CheckVals(vals);
        var site = SiteFor(kind, id, msg, file, line);
        var slot = CurrentSlot();
        lock (_lock)
        {
            if (site is null)
            {
                _dropped = Saturating.Add(_dropped, 1); // more distinct ids than the catalog holds
                return;
            }
            slot.Evals = Saturating.Add(slot.Evals, 1);
            if (ok || site.Point)
            {
                site.Trues = Saturating.Add(site.Trues, 1);
                site.FirstTrue ??= new Site.Mark(slot.Index, slot.Evals, Args.PackVals(vals));
            }
            else
            {
                site.Falses = Saturating.Add(site.Falses, 1);
                site.FirstFalse ??= new Site.Mark(slot.Index, slot.Evals, Args.PackVals(vals));
            }
        }
    }

    /// <summary>Add <paramref name="n"/> operations to this thread's work count.</summary>
    public void Work(long n)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(n);
        var slot = CurrentSlot();
        lock (_lock)
        {
            slot.Work = Saturating.Add(slot.Work, (ulong)n);
        }
    }

    /// <summary>Give the calling thread the stable key <c>label-index</c>. See <see cref="SlotTable.SetKey"/>.</summary>
    public bool NameThread(string label, long index)
    {
        SlotTable.CheckKey(label, index); // before the thread takes a slot: a refused call counts nothing
        var slot = CurrentSlot();
        lock (_lock)
        {
            return _slots.SetKey(slot, label, index);
        }
    }

    // ---- the report ----------------------------------------------------------------------------------------------

    private Rows.Witness? Witness(Site.Mark? mark) =>
        mark is null ? null : new Rows.Witness(_slots.KeyAt(mark.Slot), mark.Ord, mark.Vals);

    /// <summary>
    /// Everything the report says, read at one moment. The copies are taken with the lock held; the catalog hash, which needs a
    /// hash of every id, is worked out from the copies after it is released.
    /// </summary>
    public Snapshot Snapshot()
    {
        IReadOnlyList<Rows.AssertRow> asserts;
        IReadOnlyList<ThreadView> threads;
        int distinctIds;
        ulong dropped;
        var catalog = new List<Rows.CatalogRow>();
        lock (_lock)
        {
            foreach (var s in _sites.All())
            {
                catalog.Add(new Rows.CatalogRow(s.Id, s.Kind, s.Msg, s.File, s.Line));
            }
            var rows = new List<Rows.AssertRow>();
            foreach (var s in _sites.Canonical())
            {
                rows.Add(new Rows.AssertRow(s.Id, s.Kind, s.Trues, s.Falses, Witness(s.FirstTrue), Witness(s.FirstFalse)));
            }
            asserts = rows;
            threads = _slots.Views();
            distinctIds = _sites.DistinctIds;
            dropped = _dropped;
        }
        var entries = new List<Catalog.Entry>(catalog.Count);
        foreach (var row in catalog)
        {
            entries.Add(new Catalog.Entry(row.Id, row.Kind));
        }
        return new Snapshot(catalog, asserts, threads, Catalog.Hash(entries), distinctIds, dropped);
    }

    private long StatusFor(EndReason reason, long? statusOverride)
    {
        if (statusOverride is { } given)
        {
            return given;
        }
        lock (_lock)
        {
            // What the program recorded, else (at exit) the status the process is about to end with.
            return _status ?? (reason == EndReason.Exit ? Environment.ExitCode : 0);
        }
    }

    /// <summary>
    /// Write the report: at most once per process, and only from the process that started the SDK. Returns whether this
    /// call wrote it. <paramref name="statusOverride"/> replaces the status. The exit hook (reason <c>Exit</c>) that finds
    /// another thread still writing waits for it, so the process does not end in the middle of an explicit report written by
    /// a thread the exit does not wait for (a background thread); the wait is bounded.
    /// </summary>
    public bool Report(EndReason reason, long? statusOverride)
    {
        lock (_lock)
        {
            if (_reported)
            {
                if (reason != EndReason.Explicit)
                {
                    AwaitWriter();
                }
                return false;
            }
            _reported = true;
            _writing = true;
        }
        try
        {
            if (_sink is null || _currentPid() != _ownerPid)
            {
                return false;
            }
            Hairspring.Report.Write(Snapshot(), _identity, _sink, reason, StatusFor(reason, statusOverride), NoteDrop);
            return true;
        }
        finally
        {
            lock (_lock)
            {
                _writing = false;
                Monitor.PulseAll(_lock);
            }
        }
    }

    /// <summary>Waits (with the lock held; <see cref="Monitor.Wait(object, TimeSpan)"/> releases it) until the report being written is done, or the bound passes.</summary>
    private void AwaitWriter()
    {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        while (_writing)
        {
            var left = WriterWait - clock.Elapsed;
            if (left <= TimeSpan.Zero)
            {
                return;
            }
            Monitor.Wait(_lock, left);
        }
    }

    /// <summary>The environment a child process needs to take its tag from this one.</summary>
    public IReadOnlyDictionary<string, string> ChildEnvironment()
    {
        bool announced;
        lock (_lock)
        {
            announced = _reported;
        }
        return new Dictionary<string, string>
        {
            [Identity.EnvTag] = _identity.Tag,
            [Identity.EnvPid] = Identity.OwnerField(_ownerPid, announced, _identity.TagStable),
        };
    }
}
