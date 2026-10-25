using System;
using System.Collections.Concurrent;
using System.Collections.Generic;

namespace Hairspring;

/// <summary>
/// Assertion sites. A site is one (id, kind) pair: the same pair evaluated from several places is one site, so counts merge by
/// construction; an id used with two kinds is two sites (a catalog conflict the engine reports).
/// <para><see cref="Register"/>, <see cref="All"/>, <see cref="Canonical"/> and <see cref="DistinctIds"/> are called with the
/// recorder's lock held. <see cref="Find"/> is a single concurrent-map read and is safe without it.</para>
/// </summary>
internal sealed class SiteTable
{
    /// <summary>What the catalog can hold; ids beyond it are refused and the caller counts them as dropped.</summary>
    public const int MaxDistinct = 16384;

    /// <summary>How many spellings (as the caller wrote them) the lookup remembers, sites and refused ids together.</summary>
    public const int MaxRemembered = 2 * MaxDistinct;

    private readonly ConcurrentDictionary<string, Site>[] _raw = new ConcurrentDictionary<string, Site>[5];
    private readonly Dictionary<(string Sent, Kind Kind), Site> _bySent = new();
    private readonly HashSet<string> _ids = new(StringComparer.Ordinal);
    private readonly List<Site> _order = new();
    private int _remembered;

    public SiteTable()
    {
        for (var i = 0; i < _raw.Length; i++)
        {
            _raw[i] = new ConcurrentDictionary<string, Site>(StringComparer.Ordinal);
        }
    }

    public int RememberedCount => _remembered;

    public int DistinctIds => _ids.Count;

    /// <summary>The site the lookup remembers for this spelling, or null. No lock needed.</summary>
    public Site? Find(Kind kind, string rawId) => _raw[KindInfo.Index(kind)].TryGetValue(rawId, out var site) ? site : null;

    /// <summary>
    /// The site for this id and kind, made if it is new. Null when the table is full of other ids. The first registration's
    /// message and location stand.
    /// </summary>
    public Site? Register(Kind kind, string rawId, string msg, string file, long line)
    {
        KindInfo.Check(kind);
        Args.CheckId(rawId);
        Args.CheckMsg(msg);
        var sent = Fmt.SanitizeIdent(rawId, Fmt.IdMax);
        if (_bySent.TryGetValue((sent, kind), out var existing))
        {
            Remember(rawId, kind, existing);
            return existing;
        }
        if (!_ids.Contains(sent) && _ids.Count >= MaxDistinct)
        {
            Remember(rawId, kind, Site.Untracked);
            return null;
        }
        var site = new Site(sent, kind, msg, file, line);
        _bySent[(sent, kind)] = site;
        Remember(rawId, kind, site);
        _ids.Add(sent);
        _order.Add(site);
        return site;
    }

    /// <summary>Cache a spelling for the lock-free lookup. Past the bound it is not cached: that call takes the slow path again.</summary>
    private void Remember(string rawId, Kind kind, Site site)
    {
        if (_remembered < MaxRemembered && _raw[KindInfo.Index(kind)].TryAdd(rawId, site))
        {
            _remembered++;
        }
    }

    /// <summary>Every site in registration order.</summary>
    public IReadOnlyList<Site> All() => _order.ToArray();

    /// <summary>The first site of each id: the one whose counts the <c>assert</c> line carries.</summary>
    public IReadOnlyList<Site> Canonical()
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var first = new List<Site>();
        foreach (var site in _order)
        {
            if (seen.Add(site.Id))
            {
                first.Add(site);
            }
        }
        return first;
    }
}
