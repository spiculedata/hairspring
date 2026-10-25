using System;
using System.Diagnostics;
using System.Runtime.CompilerServices;

namespace Hairspring;

/// <summary>
/// Hairspring guest SDK for .NET: in-source property assertions for programs under test, in guest event protocol v1
/// (<c>docs/guest-protocol.md</c>). Base class library only; net10.0; Linux and macOS.
/// <code>
/// using static Hairspring.Hs;
///
/// Always(balance &gt;= 0, "bank.balance_non_negative", "no overdraft", [balance]);
/// Sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
/// Reachable("bank.recovery.ran", "recovery ran");
/// Unreachable("bank.bad_state", "never expected");
/// Work(1);                       // operations done, reported as work.ops
/// NameThread("writer", 0);       // a stable thread key "writer-0"
/// </code>
/// <para>A call site only counts in memory; the program reports once, when the process exits (or by <see cref="Report"/>). The SDK
/// starts when this class is first used (any call, or <see cref="Start"/>); a program that never touches it reports nothing.
/// .NET cannot see a call site before it runs, so a site in code the run never reaches is invisible, not unmet:
/// <see cref="Declare"/> every site whose absence matters.</para>
/// <para>A site is catalogued the first time it is evaluated. The same id and kind from several places is one site. An id is 1
/// to 128 bytes of UTF-8 (bytes outside <c>[A-Za-z0-9._:/-]</c> are sent as <c>_</c>); a message is at most 512 bytes; the values
/// are up to four, captured at the first true and the first false evaluation (missing values are 0). The file and line of a call
/// site come from the compiler. An invalid argument throws <see cref="ArgumentException"/> (or a subclass) and counts nothing.</para>
/// </summary>
public static class Hs
{
    private static readonly Recorder Recorder = Recorder.Start();

    /// <summary>Starts the SDK (it also starts at the first call) so the process reports even if no site ever runs.</summary>
    public static void Start() => RuntimeHelpers.RunClassConstructor(typeof(Hs).TypeHandle);

    /// <summary><paramref name="cond"/> must hold at every evaluation. Trial-level failure: any false. Run-level UNMET: never evaluated.</summary>
    public static void Always(
        bool cond, string id, string msg = "", ReadOnlySpan<long> vals = default,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Record(Kind.Always, cond, id, msg, vals, file, line);

    /// <summary>Like <see cref="Always"/>, but never reaching it is fine (no UNMET).</summary>
    public static void AlwaysOrUnreachable(
        bool cond, string id, string msg = "", ReadOnlySpan<long> vals = default,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Record(Kind.AlwaysOrUnreachable, cond, id, msg, vals, file, line);

    /// <summary><paramref name="cond"/> must hold at least once per run. No trial-level failure; UNMET if never true.</summary>
    public static void Sometimes(
        bool cond, string id, string msg = "", ReadOnlySpan<long> vals = default,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Record(Kind.Sometimes, cond, id, msg, vals, file, line);

    /// <summary>This point must be reached at least once per run. No trial-level failure; UNMET if never hit.</summary>
    public static void Reachable(
        string id, string msg = "", ReadOnlySpan<long> vals = default,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Record(Kind.Reachable, true, id, msg, vals, file, line);

    /// <summary>This point must never be reached. Trial-level failure: hit at least once.</summary>
    public static void Unreachable(
        string id, string msg = "", ReadOnlySpan<long> vals = default,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Record(Kind.Unreachable, true, id, msg, vals, file, line);

    /// <summary>
    /// Put a site in the catalog without evaluating it. .NET cannot see a call site that has not run, so a <c>Reachable</c> or
    /// <c>Sometimes</c> that is never reached would be invisible, and the run could not report it as unmet. Declare such sites
    /// once, early; evaluating the same id and kind later is the same site.
    /// </summary>
    public static void Declare(
        Kind kind, string id, string msg = "",
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0) =>
        Recorder.Declare(kind, id, msg, file, line);

    /// <summary>Add <paramref name="n"/> to the calling thread's operation count; the report sends the total as <c>work.ops</c>.</summary>
    public static void Work(long n) => Recorder.Work(n);

    /// <summary>
    /// Give the calling thread the key <c>label-index</c>. Returns false (and the thread keeps its claim-order key <c>t#&lt;n&gt;</c>,
    /// flagged <c>key_stable:false</c>) when another thread has that key or the thread shares the overflow slot. Throws
    /// <see cref="ArgumentException"/> for an empty label or a key over 64 bytes and <see cref="ArgumentOutOfRangeException"/> for an
    /// index outside 0 to 4294967295. Call it before the thread does anything else. Every thread that calls the SDK takes a slot,
    /// thread-pool threads included.
    /// </summary>
    public static bool NameThread(string label, long index) => Recorder.NameThread(label, index);

    /// <summary>Write the report now (reason <c>explicit</c>). The first report wins; evaluations after it are not reported.</summary>
    public static void Report() => Recorder.Report(EndReason.Explicit, null);

    /// <summary>Record <paramref name="status"/> and exit through <see cref="Environment.Exit(int)"/>, so the exit report carries it.</summary>
    public static void Exit(int status)
    {
        Recorder.SetStatus(status);
        Environment.Exit(status);
    }

    /// <summary>
    /// Puts the tag and the owning pid in the environment of a process you are about to start, so that the child (if it also
    /// uses this SDK) reports under <c>&lt;tag&gt;.s&lt;pid&gt;</c> instead of repeating this process's tag. Call it for each
    /// <see cref="ProcessStartInfo"/> before <see cref="Process.Start(ProcessStartInfo)"/>; it has no effect with
    /// <c>UseShellExecute</c>. Returns the same object.
    /// </summary>
    public static ProcessStartInfo Prepare(ProcessStartInfo startInfo)
    {
        ArgumentNullException.ThrowIfNull(startInfo);
        foreach (var (name, value) in Recorder.ChildEnvironment())
        {
            startInfo.Environment[name] = value;
        }
        return startInfo;
    }
}
