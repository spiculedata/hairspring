package hairspring;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Optional;

/** Was this process started with elevated rights? Then the events file setting is not trusted. */
final class SecureTests {
    private SecureTests() {}

    private static byte[] auxv(long... words) {
        ByteBuffer buffer = ByteBuffer.allocate(words.length * Long.BYTES).order(ByteOrder.LITTLE_ENDIAN);
        for (long word : words) {
            buffer.putLong(word);
        }
        return buffer.array();
    }

    private static byte[] status(String text) {
        return text.getBytes(java.nio.charset.StandardCharsets.US_ASCII);
    }

    static void register(Suite s) {
        s.add("secure: the AT_SECURE entry of the auxiliary vector is read", () -> {
            Check.equal(Optional.of(true), Secure.atSecureFrom(auxv(6, 4096, 23, 1, 0, 0), ByteOrder.LITTLE_ENDIAN, 8));
            Check.equal(Optional.of(false), Secure.atSecureFrom(auxv(6, 4096, 23, 0, 0, 0), ByteOrder.LITTLE_ENDIAN, 8));
        });
        s.add("secure: a vector with no AT_SECURE entry says nothing", () ->
                Check.equal(Optional.empty(), Secure.atSecureFrom(auxv(6, 4096, 0, 0), ByteOrder.LITTLE_ENDIAN, 8)));
        s.add("secure: entries after AT_NULL are not read", () ->
                Check.equal(Optional.empty(), Secure.atSecureFrom(auxv(0, 0, 23, 1), ByteOrder.LITTLE_ENDIAN, 8)));
        s.add("secure: a truncated trailing pair is ignored", () -> {
            byte[] whole = auxv(6, 4096, 23, 1);
            byte[] cut = java.util.Arrays.copyOf(whole, whole.length - 3);
            Check.equal(Optional.empty(), Secure.atSecureFrom(cut, ByteOrder.LITTLE_ENDIAN, 8));
        });
        s.add("secure: four-byte words are read too", () -> {
            ByteBuffer buffer = ByteBuffer.allocate(16).order(ByteOrder.BIG_ENDIAN);
            buffer.putInt(23).putInt(1).putInt(0).putInt(0);
            Check.equal(Optional.of(true), Secure.atSecureFrom(buffer.array(), ByteOrder.BIG_ENDIAN, 4));
        });
        s.add("secure: ids that differ between real and effective are read from the status text", () -> {
            Check.equal(Optional.of(false), Secure.idsDifferIn(status("Name:\tjava\nUid:\t1000\t1000\t1000\t1000\nGid:\t50\t50\t50\t50\n")));
            Check.equal(Optional.of(true), Secure.idsDifferIn(status("Uid:\t1000\t0\t0\t0\nGid:\t50\t50\t50\t50\n")));
            Check.equal(Optional.of(true), Secure.idsDifferIn(status("Uid:\t1000\t1000\t1000\t1000\nGid:\t50\t0\t0\t0\n")));
            Check.equal(Optional.empty(), Secure.idsDifferIn(status("Name:\tjava\n")));
            Check.equal(Optional.empty(), Secure.idsDifferIn(status("Uid:\tx\ty\nGid:\t1\t1\n")));
        });
        s.add("secure: an unreadable vector on Linux means privileged", () -> {
            Check.yes(Secure.isPrivilegedWith(() -> { throw new IOException("denied"); }, () -> status(""), true), "privileged");
        });
        s.add("secure: elsewhere there is nothing to ask and the setting is honoured", () -> {
            Check.no(Secure.isPrivilegedWith(() -> { throw new IOException("none"); }, () -> status(""), false), "privileged");
        });
        s.add("secure: the vector decides when it has the entry", () -> {
            Check.no(Secure.isPrivilegedWith(() -> auxv(23, 0, 0, 0), () -> status("Uid:\t1\t0\t0\t0\nGid:\t1\t1\t1\t1\n"), true), "AT_SECURE 0");
            Check.yes(Secure.isPrivilegedWith(() -> auxv(23, 1, 0, 0), () -> status("Uid:\t1\t1\t1\t1\nGid:\t1\t1\t1\t1\n"), true), "AT_SECURE 1");
        });
        s.add("secure: a vector without the entry falls back to comparing ids", () -> {
            Check.yes(Secure.isPrivilegedWith(() -> auxv(6, 4096, 0, 0), () -> status("Uid:\t1\t0\t0\t0\nGid:\t1\t1\t1\t1\n"), true), "ids differ");
            Check.no(Secure.isPrivilegedWith(() -> auxv(6, 4096, 0, 0), () -> status("Uid:\t1\t1\t1\t1\nGid:\t1\t1\t1\t1\n"), true), "ids equal");
            Check.yes(Secure.isPrivilegedWith(() -> auxv(6, 4096, 0, 0), () -> status(""), true), "ids unknown");
        });
    }
}
