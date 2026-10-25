using System;
using System.Buffers.Binary;
using System.IO;
using System.Text;

namespace Hairspring;

/// <summary>Was this process started with elevated rights? Then the events file setting is not trusted.</summary>
internal static class SecureTests
{
    private static byte[] Auxv(params ulong[] words)
    {
        var bytes = new byte[words.Length * 8];
        for (var i = 0; i < words.Length; i++)
        {
            BinaryPrimitives.WriteUInt64LittleEndian(bytes.AsSpan(i * 8), words[i]);
        }
        return bytes;
    }

    private static byte[] Status(string text) => Encoding.ASCII.GetBytes(text);

    private static Func<byte[]> Unreadable() => () => throw new IOException("denied");

    public static void Register(Suite s)
    {
        s.Add("secure: the AT_SECURE entry of the auxiliary vector is read", () =>
        {
            Check.Equal<bool?>(true, Secure.AtSecureFrom(Auxv(6, 4096, 23, 1, 0, 0), true, 8));
            Check.Equal<bool?>(false, Secure.AtSecureFrom(Auxv(6, 4096, 23, 0, 0, 0), true, 8));
        });
        s.Add("secure: a vector with no AT_SECURE entry says nothing", () =>
            Check.Equal<bool?>(null, Secure.AtSecureFrom(Auxv(6, 4096, 0, 0), true, 8)));
        s.Add("secure: entries after AT_NULL are not read", () =>
            Check.Equal<bool?>(null, Secure.AtSecureFrom(Auxv(0, 0, 23, 1), true, 8)));
        s.Add("secure: a truncated trailing pair is ignored", () =>
        {
            var whole = Auxv(6, 4096, 23, 1);
            Check.Equal<bool?>(null, Secure.AtSecureFrom(whole.AsSpan(0, whole.Length - 3).ToArray(), true, 8));
        });
        s.Add("secure: four-byte big-endian words are read too", () =>
        {
            var bytes = new byte[16];
            BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(0), 23);
            BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(4), 1);
            Check.Equal<bool?>(true, Secure.AtSecureFrom(bytes, false, 4));
        });
        s.Add("secure: ids that differ between real and effective are read from the status text", () =>
        {
            Check.Equal<bool?>(false, Secure.IdsDifferIn(Status("Name:\tdotnet\nUid:\t1000\t1000\t1000\t1000\nGid:\t50\t50\t50\t50\n")));
            Check.Equal<bool?>(true, Secure.IdsDifferIn(Status("Uid:\t1000\t0\t0\t0\nGid:\t50\t50\t50\t50\n")));
            Check.Equal<bool?>(true, Secure.IdsDifferIn(Status("Uid:\t1000\t1000\t1000\t1000\nGid:\t50\t0\t0\t0\n")));
            Check.Equal<bool?>(null, Secure.IdsDifferIn(Status("Name:\tdotnet\n")));
            Check.Equal<bool?>(null, Secure.IdsDifferIn(Status("Uid:\tx\ty\nGid:\t1\t1\n")));
        });
        s.Add("secure: an unreadable vector on Linux means privileged", () =>
            Check.Yes(Secure.IsPrivilegedWith(Unreadable(), () => Status(""), true), "privileged"));
        s.Add("secure: an unreadable vector counts as privileged whatever the status says", () =>
            Check.Yes(Secure.IsPrivilegedWith(Unreadable(), () => Status("Uid:\t1\t1\t1\t1\nGid:\t1\t1\t1\t1\n"), true), "privileged"));
        s.Add("secure: where no check exists the answer is the caller's", () =>
        {
            Check.No(Secure.IsPrivilegedWith(Unreadable(), () => Status(""), false), "privileged");
        });
        s.Add("secure: the vector decides when it has the entry", () =>
        {
            Check.No(Secure.IsPrivilegedWith(() => Auxv(23, 0, 0, 0), () => Status("Uid:\t1\t0\t0\t0\nGid:\t1\t1\t1\t1\n"), true), "AT_SECURE 0");
            Check.Yes(Secure.IsPrivilegedWith(() => Auxv(23, 1, 0, 0), () => Status("Uid:\t1\t1\t1\t1\nGid:\t1\t1\t1\t1\n"), true), "AT_SECURE 1");
        });
        s.Add("secure: a vector without the entry falls back to comparing ids", () =>
        {
            Check.Yes(Secure.IsPrivilegedWith(() => Auxv(6, 4096, 0, 0), () => Status("Uid:\t1\t0\t0\t0\nGid:\t1\t1\t1\t1\n"), true), "ids differ");
            Check.No(Secure.IsPrivilegedWith(() => Auxv(6, 4096, 0, 0), () => Status("Uid:\t1\t1\t1\t1\nGid:\t1\t1\t1\t1\n"), true), "ids equal");
            Check.Yes(Secure.IsPrivilegedWith(() => Auxv(6, 4096, 0, 0), () => Status(""), true), "ids unknown");
        });
    }
}
