namespace Hairspring;

/// <summary>Where finished protocol lines go. The real ones are <see cref="FdSink"/> and <see cref="StderrSink"/>; the tests use a list.</summary>
internal interface ISink
{
    /// <summary>Writes one whole line. False means the line was not (or not completely) written.</summary>
    bool WriteLine(byte[] line);

    /// <summary>Releases what the sink owns (a sink on standard error owns only its private copy).</summary>
    void Close();
}
