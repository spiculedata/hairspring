using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Numerics;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;

namespace LangAt;

/// <summary>An internal invariant failed (a worker did not start or finish, the line could not be written): exit status 3.</summary>
internal sealed class InvariantException : Exception
{
    public InvariantException(string message) : base(message)
    {
    }

    public InvariantException(string message, Exception inner) : base(message, inner)
    {
    }
}

/// <summary>What one run observed.</summary>
internal sealed record Outcome(
    int[] LockOrder,
    int[] Arrival,
    long RacyFinal,
    int[] CompletionOrder,
    ulong Rand,
    ulong ElapsedNs);

/// <summary>
/// The workload: four <see cref="Thread"/> workers that take a lock once (the lock order), run an unlocked read-spin-write on
/// a plain counter, and queue their number when they finish (the arrival order); then four tasks that each await a one
/// millisecond delay and queue their number (the completion order). Only the base class library is used.
/// </summary>
internal static class Workload
{
    public const int Threads = 4;
    public const int Rounds = 200;
    public const int SpinRounds = 1_000;

    private const ulong SpinMultiplier = 0x5851f42d4c957f2dUL;
    private const ulong SpinSeed = 0xcbf29ce484222325UL;
    private const int SpinRotation = 7;
    private const int TaskDelayMilliseconds = 1;

    /// <summary>The busy loop's result goes here, so the loop cannot be removed.</summary>
    internal static ulong Sink;

    /// <summary>A busy loop with an accumulator the compiler cannot fold away.</summary>
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

    /// <summary>Runs the workers and the tasks to completion and collects what the line reports.</summary>
    public static Outcome Run()
    {
        var start = Clock.Now();
        var shared = new SharedState();
        RunThreads(shared);
        var completion = RunTasks();
        var end = Clock.Now();
        var lockOrder = shared.LockOrderSnapshot();
        var arrival = shared.Arrival.ToArray();
        Check(lockOrder, arrival, completion);
        return new Outcome(lockOrder, arrival, shared.Racy, completion, RandomValue(), Clock.NanosBetween(start, end));
    }

    /// <summary>Every list must hold one entry per worker, or a worker did not take the lock, finish or complete.</summary>
    public static void Check(IReadOnlyCollection<int> lockOrder, IReadOnlyCollection<int> arrival, IReadOnlyCollection<int> completion)
    {
        if (lockOrder.Count != Threads || arrival.Count != Threads || completion.Count != Threads)
        {
            throw new InvariantException(
                $"a worker did not take the lock, finish or complete (lock order {lockOrder.Count}, arrival {arrival.Count}, completion {completion.Count}, want {Threads} each)");
        }
    }

    /// <summary>64 bits from the runtime's default random source.</summary>
    private static ulong RandomValue()
    {
        Span<byte> bytes = stackalloc byte[sizeof(ulong)];
        Random.Shared.NextBytes(bytes);
        return BitConverter.ToUInt64(bytes);
    }

    private static void RunThreads(SharedState shared)
    {
        var threads = new List<Thread>(Threads);
        try
        {
            for (var id = 0; id < Threads; id++)
            {
                var worker = id;
                var thread = new Thread(() => shared.Worker(worker));
                thread.Start();
                threads.Add(thread);
            }
        }
        catch (Exception e) when (e is ThreadStartException or OutOfMemoryException or InvalidOperationException)
        {
            shared.Abort();
            JoinAll(threads);
            throw new InvariantException("a worker thread failed to start", e);
        }
        JoinAll(threads);
        if (shared.Failure is { } failure)
        {
            throw new InvariantException("a worker thread failed", failure);
        }
    }

    private static void JoinAll(List<Thread> threads)
    {
        foreach (var thread in threads)
        {
            thread.Join();
        }
    }

    private static int[] RunTasks()
    {
        var completion = new ConcurrentQueue<int>();
        var tasks = new Task[Threads];
        for (var id = 0; id < Threads; id++)
        {
            var task = id;
            tasks[id] = Task.Run(async () =>
            {
                await Task.Delay(TaskDelayMilliseconds).ConfigureAwait(false);
                completion.Enqueue(task);
            });
        }
        try
        {
            Task.WaitAll(tasks);
        }
        catch (AggregateException e)
        {
            throw new InvariantException("a task failed", e);
        }
        return completion.ToArray();
    }

    /// <summary>State shared by the workers.</summary>
    private sealed class SharedState
    {
        private readonly object gate = new();
        private readonly List<int> lockOrder = new();
        private int started;
        private volatile bool abort;
        private Exception? failure;

        /// <summary>The unlocked counter: a plain field, read and written by separate operations with a busy loop between them.</summary>
        public long Racy;

        public ConcurrentQueue<int> Arrival { get; } = new();

        /// <summary>The first exception a worker died of, or null.</summary>
        public Exception? Failure => Volatile.Read(ref failure);

        public void Abort() => abort = true;

        public int[] LockOrderSnapshot()
        {
            lock (gate)
            {
                return lockOrder.ToArray();
            }
        }

        public void Worker(int id)
        {
            try
            {
                Interlocked.Increment(ref started);
                WaitForAll();
                if (abort)
                {
                    return;
                }
                lock (gate)
                {
                    lockOrder.Add(id);
                }
                RacyRounds();
                Arrival.Enqueue(id);
            }
            catch (Exception e)
            {
                // Keep the first cause, and release the other workers from the barrier.
                Interlocked.CompareExchange(ref failure, e, null);
                abort = true;
            }
        }

        private void WaitForAll()
        {
            while (Volatile.Read(ref started) < Threads && !abort)
            {
                Thread.Yield();
            }
        }

        private void RacyRounds()
        {
            for (var round = 0; round < Rounds; round++)
            {
                var seen = Racy;
                Sink = Spin(SpinRounds);
                Racy = seen + 1;
            }
        }
    }
}
