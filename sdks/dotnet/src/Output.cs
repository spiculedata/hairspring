using System;

namespace Hairspring;

/// <summary>
/// Chooses where the report goes, from the environment: <c>HAIRSPRING_EVENTS=off</c> writes nothing, a usable
/// <c>HAIRSPRING_EVENTS_FILE</c> is appended to, anything else goes to standard error. A process started with elevated rights
/// ignores the file setting, because the SDK would open a path from the environment for writing.
/// </summary>
internal static class Output
{
    public const string EnvEvents = "HAIRSPRING_EVENTS";
    public const string EnvEventsFile = "HAIRSPRING_EVENTS_FILE";
    private const string Off = "off";

    /// <summary>The sink (null when output is off) and how many losses to count as dropped (an events file that could not be opened).</summary>
    public readonly record struct Opened(ISink? Sink, ulong Dropped);

    /// <summary><paramref name="stderr"/> is called only when standard error is the destination, so no duplicate of descriptor 2 is made otherwise.</summary>
    public static Opened Open(Func<string, string?> getenv, bool privileged, Func<ISink> stderr)
    {
        if (getenv(EnvEvents) == Off)
        {
            return new Opened(null, 0);
        }
        ulong dropped = 0;
        var path = privileged ? null : getenv(EnvEventsFile);
        if (!string.IsNullOrEmpty(path))
        {
            try
            {
                return new Opened(EventsFile.Open(path), 0);
            }
            catch (Exception)
            {
                dropped = 1; // the file could not be used: events fall back to standard error, and this is counted
            }
        }
        return new Opened(stderr(), dropped);
    }
}
