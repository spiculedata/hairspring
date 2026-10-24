package hairspring;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;

/**
 * Was this process started with elevated rights? If so, {@code HAIRSPRING_EVENTS_FILE} is not trusted.
 *
 * <p>On Linux the kernel is asked ({@code AT_SECURE} in {@code /proc/self/auxv}): a set-uid or set-gid binary sets it,
 * and so do file capabilities and a security-module transition, which change no id. A vector that cannot be read counts
 * as privileged. A vector that reads but has no {@code AT_SECURE} entry falls back to comparing real and effective ids
 * in {@code /proc/self/status}. Elsewhere the JVM offers no effective id, so nothing can be asked and the setting is
 * honoured.
 */
final class Secure {
    private static final long AT_NULL = 0;
    private static final long AT_SECURE = 23;
    private static final Path AUXV = Path.of("/proc/self/auxv");
    private static final Path STATUS = Path.of("/proc/self/status");
    private static final int WORD_32 = 4;
    private static final int WORD_64 = 8;

    private Secure() {}

    /** Reads a file of the process's own, which may fail. */
    interface Reader {
        byte[] read() throws IOException;
    }

    private static long word(ByteBuffer buffer, int size) {
        return size == WORD_32 ? buffer.getInt() & 0xffffffffL : buffer.getLong();
    }

    /** The auxiliary vector: pairs of machine words (type, value), ended by type 0. Empty when there is no AT_SECURE entry. */
    static Optional<Boolean> atSecureFrom(byte[] raw, ByteOrder order, int wordSize) {
        ByteBuffer buffer = ByteBuffer.wrap(raw).order(order);
        while (buffer.remaining() >= 2 * wordSize) {
            long kind = word(buffer, wordSize);
            long value = word(buffer, wordSize);
            if (kind == AT_NULL) {
                break;
            }
            if (kind == AT_SECURE) {
                return Optional.of(value != 0);
            }
        }
        return Optional.empty();
    }

    private static Optional<long[]> idPair(String status, String name) {
        for (String line : status.split("\n")) {
            if (line.startsWith(name + ":")) {
                String[] fields = line.substring(name.length() + 1).trim().split("\\s+");
                if (fields.length < 2) {
                    return Optional.empty();
                }
                try {
                    return Optional.of(new long[] {Long.parseLong(fields[0]), Long.parseLong(fields[1])});
                } catch (NumberFormatException unreadable) {
                    return Optional.empty();
                }
            }
        }
        return Optional.empty();
    }

    /** Whether the real and effective user or group id differ, from the text of {@code /proc/self/status}; empty if unreadable. */
    static Optional<Boolean> idsDifferIn(byte[] statusText) {
        String text = new String(statusText, StandardCharsets.US_ASCII);
        Optional<long[]> uid = idPair(text, "Uid");
        Optional<long[]> gid = idPair(text, "Gid");
        if (uid.isEmpty() || gid.isEmpty()) {
            return Optional.empty();
        }
        return Optional.of(uid.get()[0] != uid.get()[1] || gid.get()[0] != gid.get()[1]);
    }

    static boolean isPrivilegedWith(Reader auxv, Reader status, boolean linux, ByteOrder order, int wordSize) {
        if (!linux) {
            return false;
        }
        try {
            Optional<Boolean> secure = atSecureFrom(auxv.read(), order, wordSize);
            if (secure.isPresent()) {
                return secure.get();
            }
            return idsDifferIn(status.read()).orElse(true);
        } catch (IOException unreadable) {
            return true; // the case AT_SECURE exists for: a vector the process may not read
        }
    }

    static boolean isPrivilegedWith(Reader auxv, Reader status, boolean linux) {
        return isPrivilegedWith(auxv, status, linux, ByteOrder.LITTLE_ENDIAN, WORD_64);
    }

    static boolean isPrivileged() {
        boolean linux = System.getProperty("os.name", "").startsWith("Linux");
        int wordSize = "32".equals(System.getProperty("sun.arch.data.model")) ? WORD_32 : WORD_64;
        return isPrivilegedWith(
                () -> Files.readAllBytes(AUXV), () -> Files.readAllBytes(STATUS), linux, ByteOrder.nativeOrder(), wordSize);
    }
}
