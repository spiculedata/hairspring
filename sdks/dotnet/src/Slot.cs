namespace Hairspring;

/// <summary>One thread's counters. Every field is changed only with the recorder's lock held.</summary>
internal sealed class Slot
{
    public Slot(int index, string key)
    {
        Index = index;
        Key = key;
    }

    public int Index { get; }

    public string Key { get; set; }

    public bool Stable { get; set; }

    public ulong Evals { get; set; }

    public ulong Work { get; set; }
}
