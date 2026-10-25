using System.Globalization;

namespace LangAt;

/// <summary>The one stdout line of the program, in the form every language's fixture prints, formatted the same under every culture.</summary>
internal static class LangAtLine
{
    private const int CountWidth = 20;

    private static string Padded(long value) => value.ToString(CultureInfo.InvariantCulture).PadLeft(CountWidth, '0');

    private static string Hex(ulong value) => value.ToString("x16", CultureInfo.InvariantCulture);

    /// <summary><c>LANGAT v1 lang=dotnet mode=&lt;mode&gt; final=&lt;20 digits&gt; expect=&lt;20 digits&gt; order=&lt;16 hex&gt; rand=&lt;16 hex&gt;</c> and a newline.</summary>
    public static string Format(Mode mode, long finalValue, ulong order, ulong rand) =>
        "LANGAT v1 lang=" + Contract.Lang +
        " mode=" + mode.Wire() +
        " final=" + Padded(finalValue) +
        " expect=" + Padded(Contract.Expected) +
        " order=" + Hex(order) +
        " rand=" + Hex(rand) +
        "\n";
}
