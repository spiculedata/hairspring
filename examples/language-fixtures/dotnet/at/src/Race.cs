using System;
using System.Numerics;
using System.Runtime.CompilerServices;
using System.Threading;
using Hairspring;

namespace LangAt;

/// <summary>
/// The planted race: four threads behind a start barrier, each doing <c>iterations</c> times a load of one shared counter, a
/// busy loop and a store of the value read plus one.
/// <para>The window is the three steps in <see cref="RacyIterations"/> (or <see cref="LockedIterations"/>, which holds a lock
/// across them): <c>var v = _counter; var acc = Spin(window); _counter = v + 1;</c>. <c>_counter</c> is a plain field (no
/// <c>Volatile</c>, no <c>Interlocked</c>), so the load and the store are two separate operations and an update is lost whenever
/// another worker runs between them. There is no system call, allocation, lock, sleep, yield, clock read or SDK call between
/// them; the SDK's work counter and the busy loop's result are written after the store.</para>
/// <para>Why the window does not depend on the JIT. <see cref="Spin"/> is marked <c>NoInlining</c>, so the compiler sees an
/// opaque call between the load and the store: it cannot move the load past the call or keep the counter in a register across
/// it, because the call might write the field. That holds for every tier of the JIT, which is why the window is not tied to
/// a count of calls the way a tiering interpreter's would be. The busy loop's result is returned and stored in a static
/// field after the store, so the loop cannot be dropped, and it is plain arithmetic (a rotation, an exclusive or and a
/// multiplication per round, no library call).</para>
/// </summary>
internal sealed class Race
{
    private const ulong SpinMultiplier = 0x5851f42d4c957f2dUL;

    /// <summary>A fixed non-zero start, so the accumulator never sticks at zero.</summary>
    private const ulong SpinSeed = 0xcbf29ce484222325UL;

    private const int SpinRotation = 7;

    /// <summary>Where the busy loop's result goes, so the loop has an effect. Written after the store, outside the window.</summary>
    private static ulong _sink;

    private readonly Mode _mode;
    private readonly int _iterations;
    private readonly object _gate = new();
    private readonly StartBarrier _barrier;
    private readonly int[] _arrival = new int[Contract.Threads];
    private int _started;
    private int _arrivals;
    private string? _failure;

    /// <summary>The shared counter: a plain, non-volatile field.</summary>
    private long _counter;

    private Race(Mode mode, int iterations)
    {
        _mode = mode;
        _iterations = iterations;
        _barrier = new StartBarrier(() => Volatile.Read(ref _started), Contract.Threads);
    }

    /// <summary>A busy loop in this file's own code: <paramref name="rounds"/> rounds on an accumulator the caller keeps.</summary>
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static ulong Spin(int rounds)
    {
        var acc = SpinSeed;
        for (var i = 0; i < rounds; i++)
        {
            acc = unchecked((BitOperations.RotateLeft(acc, SpinRotation) ^ (ulong)i) * SpinMultiplier);
        }
        return acc;
    }

    /// <summary>Runs the four workers to completion.</summary>
    public static RaceResult Run(Mode mode, int iterations) => new Race(mode, iterations).Execute();

    private void RacyIterations()
    {
        var window = _mode.WindowRounds();
        for (var i = 0; i < _iterations; i++)
        {
            var v = _counter;
            var acc = Spin(window);
            _counter = v + 1;
            _sink = acc;
            Hs.Work(1);
        }
    }

    private void LockedIterations()
    {
        var window = _mode.WindowRounds();
        for (var i = 0; i < _iterations; i++)
        {
            ulong acc;
            lock (_gate)
            {
                var v = _counter;
                acc = Spin(window);
                _counter = v + 1;
            }
            _sink = acc;
            Hs.Work(1);
        }
    }

    private void Worker(int index)
    {
        try
        {
            Hs.NameThread("worker", index);
            Interlocked.Increment(ref _started);
            if (!_barrier.Await())
            {
                return;
            }
            if (_mode.IsLocked())
            {
                LockedIterations();
            }
            else
            {
                RacyIterations();
            }
            _arrival[Interlocked.Increment(ref _arrivals) - 1] = index;
        }
        catch (Exception problem)
        {
            Volatile.Write(ref _failure, $"worker {index}: {problem}");
            _barrier.Abort();
        }
    }

    private RaceResult Execute()
    {
        var threads = new Thread[Contract.Threads];
        var spawned = 0;
        try
        {
            for (var index = 0; index < Contract.Threads; index++)
            {
                var worker = index;
                threads[index] = new Thread(() => Worker(worker)) { Name = "worker-" + index };
                threads[index].Start();
                spawned++;
            }
        }
        catch (Exception refused) when (refused is OutOfMemoryException or ThreadStartException or InvalidOperationException)
        {
            Volatile.Write(ref _failure, $"a thread did not start: {refused.Message}");
            _barrier.Abort();
        }
        for (var i = 0; i < spawned; i++)
        {
            threads[i].Join();
        }
        if (Volatile.Read(ref _failure) is { } failure)
        {
            throw new RaceFailure(failure);
        }
        if (spawned != Contract.Threads || Volatile.Read(ref _arrivals) != Contract.Threads)
        {
            throw new RaceFailure("a worker thread failed to start or died");
        }
        return new RaceResult(_counter, Volatile.Read(ref _started), (int[])_arrival.Clone());
    }
}
