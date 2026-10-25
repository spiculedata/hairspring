namespace LangAt;

/// <summary>
/// The three modes of the one program, chosen by the first argument and never by a build flag: <c>buggy</c> (the planted
/// race), <c>fixed</c> (the same three steps under a lock) and <c>wide</c> (<c>buggy</c> with a busy loop ten times longer).
/// </summary>
internal sealed class Mode
{
    public static readonly Mode Buggy = new("buggy", Contract.Window, false);
    public static readonly Mode Fixed = new("fixed", Contract.Window, true);
    public static readonly Mode Wide = new("wide", Contract.WindowWide, false);

    private static readonly Mode[] All = [Buggy, Fixed, Wide];

    private readonly string _wire;
    private readonly int _window;
    private readonly bool _locked;

    private Mode(string wire, int window, bool locked)
    {
        _wire = wire;
        _window = window;
        _locked = locked;
    }

    public string Wire() => _wire;

    /// <summary>Rounds of the busy loop between the load and the store.</summary>
    public int WindowRounds() => _window;

    /// <summary>Whether the load, the busy loop and the store sit inside a lock.</summary>
    public bool IsLocked() => _locked;

    /// <summary>The mode named by exactly one argument, or null for a usage error.</summary>
    public static Mode? Parse(string[] args)
    {
        if (args.Length != 1)
        {
            return null;
        }
        foreach (var mode in All)
        {
            if (mode._wire == args[0])
            {
                return mode;
            }
        }
        return null;
    }
}
