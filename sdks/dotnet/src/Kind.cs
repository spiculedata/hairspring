using System;

namespace Hairspring;

/// <summary>
/// The five assertion kinds. The numbers are the C SDK's (<c>enum hs_kind</c>): they are part of the catalog hash.
/// <code>
/// kind                  trial-level failure     run-level UNMET
/// Always                any evaluation false    never evaluated
/// AlwaysOrUnreachable   any evaluation false    none
/// Sometimes             none                    never true
/// Reachable             none                    never hit
/// Unreachable           hit at least once       none
/// </code>
/// </summary>
public enum Kind
{
    /// <summary>The condition must hold at every evaluation.</summary>
    Always = 1,

    /// <summary>Like <see cref="Always"/>, but never reaching it is fine.</summary>
    AlwaysOrUnreachable = 2,

    /// <summary>The condition must hold at least once per run.</summary>
    Sometimes = 3,

    /// <summary>The point must be reached at least once per run.</summary>
    Reachable = 4,

    /// <summary>The point must never be reached.</summary>
    Unreachable = 5,
}

/// <summary>What the protocol says about each <see cref="Kind"/>.</summary>
internal static class KindInfo
{
    /// <summary>The C SDK's number for the kind.</summary>
    public static int Number(Kind kind) => (int)kind;

    /// <summary>The kind's name on the wire (<c>type</c>).</summary>
    public static string WireName(Kind kind) => kind switch
    {
        Kind.Always => "always",
        Kind.AlwaysOrUnreachable => "always_or_unreachable",
        Kind.Sometimes => "sometimes",
        Kind.Reachable => "reachable",
        Kind.Unreachable => "unreachable",
        _ => throw new ArgumentOutOfRangeException(nameof(kind), kind, "not an assertion kind"),
    };

    /// <summary>Whether the run-level result is UNMET when the site never succeeds (<c>must_hit</c>).</summary>
    public static bool MustHit(Kind kind) => kind is Kind.Always or Kind.Sometimes or Kind.Reachable;

    /// <summary>A point has no condition: reaching it is the event (<c>Reachable</c>, <c>Unreachable</c>).</summary>
    public static bool IsPoint(Kind kind) => kind is Kind.Reachable or Kind.Unreachable;

    /// <summary>Throws <see cref="ArgumentOutOfRangeException"/> for a value that is not one of the five kinds.</summary>
    public static void Check(Kind kind)
    {
        if (kind < Kind.Always || kind > Kind.Unreachable)
        {
            throw new ArgumentOutOfRangeException(nameof(kind), kind, "not an assertion kind");
        }
    }

    /// <summary>The index of the kind in a table of five.</summary>
    public static int Index(Kind kind) => (int)kind - 1;
}
