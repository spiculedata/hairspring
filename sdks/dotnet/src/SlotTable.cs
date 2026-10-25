using System;
using System.Collections.Generic;
using System.Globalization;

namespace Hairspring;

/// <summary>
/// The table of per-thread slots. A thread claims one on its first SDK call; slots are never recycled and keys cannot be
/// reused, so the table suits fixed pools. Past the 255th thread the rest share the last slot.
/// <para>The table does no locking of its own: the recorder calls it with its lock held.</para>
/// </summary>
internal sealed class SlotTable
{
    public const int MaxThreads = 256;
    public const int OverflowSlot = MaxThreads - 1;
    public const string OverflowKey = "t#overflow";
    public const long MaxIndex = 4294967295L;

    /// <summary>The slot a claim got; <c>Shared</c> says the thread shares the overflow slot.</summary>
    public readonly record struct Claimed(Slot Slot, bool Shared);

    private readonly List<Slot> _slots = new();
    private long _claims;

    public int Used => _slots.Count;

    public Claimed Claim()
    {
        var n = _claims < long.MaxValue ? _claims++ : _claims;
        if (n < OverflowSlot)
        {
            var slot = new Slot((int)n, "t#" + (n + 1).ToString(CultureInfo.InvariantCulture));
            _slots.Add(slot);
            return new Claimed(slot, false);
        }
        if (n == OverflowSlot)
        {
            _slots.Add(new Slot(OverflowSlot, OverflowKey));
        }
        return new Claimed(_slots[OverflowSlot], true);
    }

    public string KeyAt(int index) => _slots.Count == 0 ? "" : _slots[Math.Min(index, _slots.Count - 1)].Key;

    /// <summary>
    /// Give <paramref name="slot"/> the key <c>label-index</c>. Bytes of the label outside <c>[A-Za-z0-9._:/-]</c> become
    /// <c>_</c>. An empty label or a key over 64 bytes is a programming error; a key another thread has, or a thread that
    /// shares the overflow slot, is refused with <c>false</c> and the thread keeps its claim-order key.
    /// </summary>
    public bool SetKey(Slot slot, string label, long index)
    {
        ArgumentNullException.ThrowIfNull(label);
        if (label.Length == 0)
        {
            throw new ArgumentException("a thread label must not be empty", nameof(label));
        }
        if (label.Contains('\0', StringComparison.Ordinal))
        {
            // The wire form stops at a NUL, so the key would be sent shortened.
            throw new ArgumentException("a thread label must not contain a NUL character", nameof(label));
        }
        if (index < 0 || index > MaxIndex)
        {
            throw new ArgumentOutOfRangeException(nameof(index), index, $"a thread index must be between 0 and {MaxIndex}");
        }
        var key = Fmt.SanitizeIdent(label, Fmt.Utf8(label).Length) + "-" + index.ToString(CultureInfo.InvariantCulture);
        if (key.Length > Fmt.ThreadKeyMax)
        {
            throw new ArgumentException($"the key {key} is longer than {Fmt.ThreadKeyMax} bytes", nameof(label));
        }
        if (slot.Index == OverflowSlot)
        {
            return false;
        }
        foreach (var other in _slots)
        {
            if (!ReferenceEquals(other, slot) && other.Key == key)
            {
                return false;
            }
        }
        slot.Key = key;
        slot.Stable = true;
        return true;
    }

    public IReadOnlyList<ThreadView> Views()
    {
        var views = new List<ThreadView>(_slots.Count);
        foreach (var s in _slots)
        {
            views.Add(new ThreadView(s.Key, s.Evals, s.Work, 0, s.Stable));
        }
        return views;
    }
}
