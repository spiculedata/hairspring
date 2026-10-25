namespace Hairspring;

/// <summary>An immutable copy of a <see cref="Slot"/>, for the report.</summary>
internal sealed record ThreadView(string Key, ulong Evals, ulong Work, ulong TapeConsumed, bool Stable);
