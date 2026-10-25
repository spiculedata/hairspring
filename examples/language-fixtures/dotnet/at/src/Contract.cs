namespace LangAt;

/// <summary>The constants every language's fixture shares: four workers, a thousand iterations, a busy loop of a thousand rounds.</summary>
internal static class Contract
{
    public const string Lang = "dotnet";
    internal const int Threads = 4;
    internal const int Iterations = 1000;
    internal const int Window = 1000;
    internal const int WindowWide = 10000;
    internal const int Expected = Threads * Iterations;

    internal const int ExitOk = 0;
    internal const int ExitInvariant = 3;
    internal const int ExitUsage = 64;
}
