namespace Hairspring;

/// <summary>Why a process's report ends: normal exit, a fatal signal or an explicit <c>Report()</c>.</summary>
internal enum EndReason
{
    Exit,
    Signal,
    Explicit,
}

internal static class EndReasonInfo
{
    public static string Wire(EndReason reason) => reason switch
    {
        EndReason.Exit => "exit",
        EndReason.Signal => "signal",
        _ => "explicit",
    };
}
