using System;
using System.Threading;

namespace LangAt;

/// <summary>
/// The start barrier: each worker first counts itself in <c>started</c>, then waits here until every party has. It yields the
/// processor while it waits (a system call, which the contract allows before the first iteration) and has no timeout: a
/// worker that fails, or a thread that cannot be started, aborts it, which wakes the others and tells them the start failed.
/// </summary>
internal sealed class StartBarrier
{
    private readonly Func<int> _started;
    private readonly int _parties;
    private volatile bool _aborted;

    public StartBarrier(Func<int> started, int parties)
    {
        _started = started;
        _parties = parties;
    }

    /// <summary>Waits until <c>started</c> reaches the number of parties. True when it did, false when the barrier was aborted.</summary>
    public bool Await()
    {
        while (_started() < _parties && !_aborted)
        {
            Thread.Yield();
        }
        return !_aborted;
    }

    public void Abort()
    {
        _aborted = true;
    }
}
