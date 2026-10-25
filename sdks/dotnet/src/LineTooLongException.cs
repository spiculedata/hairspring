using System;

namespace Hairspring;

/// <summary>A protocol line would pass the 4096 byte cap. A line is refused whole, never truncated.</summary>
internal sealed class LineTooLongException : Exception
{
    public LineTooLongException(string message) : base(message)
    {
    }
}
