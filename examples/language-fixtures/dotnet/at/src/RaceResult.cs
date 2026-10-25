using System;

namespace LangAt;

/// <summary>What a run of the workers leaves: the final counter, how many workers started and the order they finished in.</summary>
internal sealed record RaceResult(long FinalValue, int Started, int[] Arrival);

/// <summary>An internal invariant of the program failed (a worker did not start or died, the barrier broke): exit status 3.</summary>
internal sealed class RaceFailure : Exception
{
    public RaceFailure(string message) : base(message)
    {
    }
}
