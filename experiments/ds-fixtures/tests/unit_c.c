/* Unit checks of the pure helpers in c/ds_common.h, built and run by tests/test_c.py (never part of an experiment).
 *
 *   unit_c logic          orphan-guard arithmetic, request classification, name and tag validation
 *   unit_c emit N         write one line of exactly N bytes including the newline (N up to 8192)
 *   unit_c fmt N          format a path of N characters into a 64-byte buffer through ds_fmt_or_die
 */
#include "../c/ds_common.h"

static int failures;

#define EXPECT(cond)                                                      \
    do {                                                                  \
        if (!(cond)) {                                                    \
            fprintf(stderr, "FAIL line %d: %s\n", __LINE__, #cond);       \
            failures++;                                                   \
        }                                                                 \
    } while (0)

static int run_logic(void) {
    /* The silence guard: strictly more than the limit, never on a clock that went backwards. */
    EXPECT(!ds_idle_expired(120000, 0, DS_IDLE_LIMIT_MS));
    EXPECT(ds_idle_expired(120001, 0, DS_IDLE_LIMIT_MS));
    EXPECT(!ds_idle_expired(5, 10, DS_IDLE_LIMIT_MS));
    EXPECT(!ds_idle_expired(130000, 10001, DS_IDLE_LIMIT_MS));
    EXPECT(ds_idle_expired(130002, 10001, DS_IDLE_LIMIT_MS));
    EXPECT(DS_IDLE_LIMIT_MS == 120000ULL);
    /* The wait-count guard. */
    EXPECT(!ds_idle_waits_expired(0));
    EXPECT(!ds_idle_waits_expired(DS_MAX_IDLE_WAITS - 1));
    EXPECT(ds_idle_waits_expired(DS_MAX_IDLE_WAITS));
    EXPECT(DS_MAX_IDLE_WAITS == 2400u);
    /* Only requests count as activity; heartbeats and HELLO do not. */
    const char *requests[] = {"PUT", "GET", "REPL", "QUIT"};
    for (size_t i = 0; i < 4; i++) EXPECT(ds_counts_as_request(requests[i]));
    const char *quiet[] = {"HB", "HELLO", "", "put", "BOGUS"};
    for (size_t i = 0; i < 5; i++) EXPECT(!ds_counts_as_request(quiet[i]));
    /* Names that reach a path. */
    EXPECT(ds_valid_name("n1") && ds_valid_name("abc123") && ds_valid_name("A"));
    EXPECT(!ds_valid_name("") && !ds_valid_name("../x") && !ds_valid_name("n/1") && !ds_valid_name("a.b"));
    EXPECT(!ds_valid_name("abcdefghijklmnopq"));
    EXPECT(ds_valid_tag("n1.i0", "n1") && ds_valid_tag("n1.i123456789", "n1"));
    EXPECT(!ds_valid_tag("n1.i", "n1") && !ds_valid_tag("n2.i0", "n1") && !ds_valid_tag("n1.i0/x", "n1"));
    EXPECT(!ds_valid_tag("n1.i1234567890", "n1") && !ds_valid_tag("../n1.i0", "n1") && !ds_valid_tag("n1.i-1", "n1"));
    return failures ? 1 : 0;
}

int main(int argc, char **argv) {
    if (argc < 2) return 64;
    if (strcmp(argv[1], "logic") == 0) return run_logic();
    if (argc < 3) return 64;
    int n = atoi(argv[2]);
    if (strcmp(argv[1], "emit") == 0) {
        char text[8200];
        if (n < 1 || n > 8192) return 64;
        memset(text, 'x', (size_t)n - 1);
        text[n - 1] = '\0';
        ds_emit("%s\n", text);
        return 0;
    }
    if (strcmp(argv[1], "fmt") == 0) {
        char text[8200], out[64];
        if (n < 1 || n > 8192) return 64;
        memset(text, 'p', (size_t)n);
        text[n] = '\0';
        ds_fmt_or_die(out, sizeof out, "%s", text);
        ds_emit("ok\n");
        return 0;
    }
    return 64;
}
