using System;
using System.Globalization;

namespace Hairspring;

/// <summary>
/// The process tag, and how it survives a spawn.
/// <para>
/// The tag travels in the environment as <c>HAIRSPRING_PROC</c>, with the owning pid and two flags in
/// <c>HAIRSPRING_PROC_PID</c> (<c>&lt;pid&gt;</c>, <c>&lt;pid&gt;:a</c>, <c>&lt;pid&gt;:u</c>, <c>&lt;pid&gt;:au</c>; <c>a</c> = the
/// tag has been announced with a report, <c>u</c> = the tag is not stable across runs):
/// </para>
/// <list type="bullet">
/// <item>root process: <c>HAIRSPRING_PROC</c> if the harness set it, else <c>p1</c>;</item>
/// <item>spawned child: the inherited pid is not its own, so the inherited tag belongs to an ancestor and the child takes
/// <c>&lt;inherited&gt;.s&lt;pid&gt;</c>, flagged unstable (a pid differs between runs);</item>
/// <item>an image that owns the inherited pid and whose tag was announced takes <c>&lt;tag&gt;.x1</c> (a native launcher that
/// execs a .NET program).</item>
/// </list>
/// <para>There is no fork handler: managed code does not survive a fork. A forked child never reports (see the owner check).</para>
/// </summary>
internal sealed record Identity(string Tag, bool TagStable, bool Refused)
{
    public const string EnvTag = "HAIRSPRING_PROC";
    public const string EnvPid = "HAIRSPRING_PROC_PID";
    public const string DefaultTag = "p1";
    public const string ReservedTag = "_eval";

    /// <summary>The readable part kept when a derived tag would pass 64 bytes.</summary>
    private const int HashPrefixMax = 40;

    private const ulong HashMix = 0x9E3779B97F4A7C15UL;
    private const long NoPid = -1;

    /// <summary>1 to 64 bytes of <c>[A-Za-z0-9._:/-]</c>, and not <c>_eval</c>, which is reserved for <c>eval</c> lines.</summary>
    public static bool TagValid(string tag)
    {
        var data = Fmt.Utf8(tag);
        if (data.Length == 0 || data.Length > Fmt.TagMax || tag == ReservedTag)
        {
            return false;
        }
        foreach (var b in data)
        {
            if (!Fmt.IsIdentChar(b))
            {
                return false;
            }
        }
        return true;
    }

    /// <summary><c>base.&lt;letter&gt;&lt;n&gt;</c>, or <c>&lt;first 40 bytes of base&gt;.h&lt;16 hex digits&gt;</c> when that would not fit in a tag.</summary>
    public static string Derive(string baseTag, char letter, long n)
    {
        var plain = baseTag + "." + letter + n.ToString(CultureInfo.InvariantCulture);
        if (plain.Length <= Fmt.TagMax)
        {
            return plain;
        }
        var keep = Math.Min(baseTag.Length, HashPrefixMax);
        var mix = Fmt.IdentHash(baseTag, Fmt.TagMax) ^ unchecked((ulong)n * HashMix) ^ letter;
        return baseTag[..keep] + ".h" + Fmt.Hex64(mix);
    }

    /// <summary>The value of <c>HAIRSPRING_PROC_PID</c>.</summary>
    public static string OwnerField(long pid, bool announced, bool stable)
    {
        var number = pid.ToString(CultureInfo.InvariantCulture);
        if (!announced && stable)
        {
            return number;
        }
        return number + ":" + (announced ? "a" : "") + (stable ? "" : "u");
    }

    private sealed record Owner(long Pid, bool Announced, bool Unstable);

    private static bool IsAsciiDigit(char c) => c is >= '0' and <= '9';

    /// <summary><c>&lt;pid&gt;</c> or <c>&lt;pid&gt;:</c> followed by the flags <c>a</c> and <c>u</c>; anything else is not an owner field.</summary>
    private static Owner? ParseOwner(string text)
    {
        var digits = 0;
        while (digits < text.Length && IsAsciiDigit(text[digits]))
        {
            digits++;
        }
        if (digits == 0)
        {
            return null;
        }
        // A number too big for a pid is a number no pid equals; the field is still well formed.
        var pid = long.TryParse(text.AsSpan(0, digits), NumberStyles.None, CultureInfo.InvariantCulture, out var parsed) ? parsed : NoPid;
        var rest = text[digits..];
        if (rest.Length == 0)
        {
            return new Owner(pid, false, false);
        }
        if (rest[0] != ':')
        {
            return null;
        }
        for (var i = 1; i < rest.Length; i++)
        {
            if (rest[i] != 'a' && rest[i] != 'u')
            {
                return null;
            }
        }
        return new Owner(pid, rest.Contains('a', StringComparison.Ordinal), rest.Contains('u', StringComparison.Ordinal));
    }

    /// <summary>Decide the tag from the inherited environment values and this process's pid.</summary>
    public static Identity Resolve(string? inheritedTag, string? owner, long pid)
    {
        if (string.IsNullOrEmpty(inheritedTag))
        {
            return new Identity(DefaultTag, true, false);
        }
        if (!TagValid(inheritedTag))
        {
            return new Identity(DefaultTag, true, true);
        }
        if (owner is null)
        {
            return new Identity(inheritedTag, true, false);
        }
        var parsed = ParseOwner(owner);
        if (parsed is not null && parsed.Pid == pid)
        {
            var tag = parsed.Announced ? Derive(inheritedTag, 'x', 1) : inheritedTag;
            return new Identity(tag, !parsed.Unstable, false);
        }
        return new Identity(Derive(inheritedTag, 's', pid), false, false);
    }
}
