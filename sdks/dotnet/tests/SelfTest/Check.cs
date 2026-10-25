using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;

namespace Hairspring;

/// <summary>A failed check: thrown by <see cref="Check"/>, reported by <see cref="Suite"/>.</summary>
internal sealed class CheckFailedException : Exception
{
    public CheckFailedException(string message) : base(message)
    {
    }
}

/// <summary>Assertions for the self-test.</summary>
internal static class Check
{
    public static void Equal<T>(T want, T got) => Equal("values", want, got);

    public static void Equal<T>(string what, T want, T got)
    {
        if (!Same(want, got))
        {
            throw new CheckFailedException($"{what}: want {Show(want)}, got {Show(got)}");
        }
    }

    public static void Yes(bool condition, string what)
    {
        if (!condition)
        {
            throw new CheckFailedException($"not true: {what}");
        }
    }

    public static void No(bool condition, string what) => Yes(!condition, $"not ({what})");

    /// <summary>Runs <paramref name="body"/>, which must throw exactly <typeparamref name="T"/> (or a subclass); returns it.</summary>
    public static T Raises<T>(Action body) where T : Exception
    {
        try
        {
            body();
        }
        catch (Exception thrown)
        {
            if (thrown is T wanted)
            {
                return wanted;
            }
            throw new CheckFailedException($"want {typeof(T).Name}, got {thrown}");
        }
        throw new CheckFailedException($"want {typeof(T).Name}, nothing was thrown");
    }

    private static bool Same<T>(T want, T got)
    {
        if (want is IEnumerable wantItems && got is IEnumerable gotItems && want is not string)
        {
            return wantItems.Cast<object?>().SequenceEqual(gotItems.Cast<object?>());
        }
        return EqualityComparer<T>.Default.Equals(want, got);
    }

    private static string Show<T>(T value)
    {
        if (value is string text)
        {
            return $"\"{text}\"";
        }
        if (value is IEnumerable items)
        {
            return "[" + string.Join(", ", items.Cast<object?>().Select(i => i?.ToString())) + "]";
        }
        return value?.ToString() ?? "null";
    }
}
