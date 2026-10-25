using System;
using System.Collections.Generic;

namespace Hairspring;

/// <summary>A list of named test cases and a runner that prints one line per case.</summary>
internal sealed class Suite
{
    private readonly List<(string Name, Action Body)> _cases = new();

    public void Add(string name, Action body) => _cases.Add((name, body));

    public int Size => _cases.Count;

    /// <summary>Runs every case (after the optional name filter); returns how many failed.</summary>
    public int Run(System.IO.TextWriter output, string? filter)
    {
        var failed = 0;
        var ran = 0;
        foreach (var (name, body) in _cases)
        {
            if (filter is not null && !name.Contains(filter, StringComparison.Ordinal))
            {
                continue;
            }
            ran++;
            try
            {
                body();
                output.WriteLine($"ok   {name}");
            }
            catch (Exception problem)
            {
                failed++;
                output.WriteLine(problem is CheckFailedException ? $"FAIL {name}: {problem.Message}" : $"FAIL {name}: {problem}");
            }
        }
        output.WriteLine($"{ran - failed} passed, {failed} failed");
        return failed;
    }
}
