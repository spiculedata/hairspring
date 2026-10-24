using System;
using System.Diagnostics;

namespace LangAt;

/// <summary>
/// The probe's only clock: the monotonic stopwatch counter, read at the start and at the end of the run. The line carries
/// the difference (vtime_ns). Nothing else in the program reads a clock.
/// </summary>
internal static class Clock
{
    private const ulong NanosPerSecond = 1_000_000_000UL;

    /// <summary>The monotonic counter now, in stopwatch ticks.</summary>
    public static long Ticks() => Stopwatch.GetTimestamp();

    /// <summary>Nanoseconds between two readings of <see cref="Ticks"/>, using the stopwatch's own tick frequency.</summary>
    public static ulong NanosBetween(long start, long end) => NanosBetween(start, end, Stopwatch.Frequency);

    /// <summary>
    /// Nanoseconds between two tick readings at <paramref name="ticksPerSecond"/>. A reading that went backwards is zero;
    /// a difference that does not fit in 64 bits saturates instead of wrapping.
    /// </summary>
    public static ulong NanosBetween(long start, long end, long ticksPerSecond)
    {
        if (end <= start || ticksPerSecond <= 0)
        {
            return 0;
        }
        var nanos = (UInt128)(ulong)(end - start) * NanosPerSecond / (ulong)ticksPerSecond;
        return nanos > ulong.MaxValue ? ulong.MaxValue : (ulong)nanos;
    }
}
