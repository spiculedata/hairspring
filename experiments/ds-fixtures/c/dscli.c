/* dscli: the DS client. Issues a fixed operation list against the servers and records an invoke/return history
 * ordered by a shared sequence counter.
 *
 *   dscli --node=c1 --servers=P1[,P2,P3] --tmp=DIR [--per-op-conn] [--reconnect] [--pace-ms=N]
 *   dscli --node=c1 [--reconnect] --print-ops          (print the operation list and exit; no network)
 *
 * Normal mode (loopback experiment): 40 operations, PUT or GET, spread over the servers. --per-op-conn opens one connection per
 * operation (for the blocking-socket topology).
 * --reconnect (crash experiment): 60 PUT operations against one node; on end-of-file or reset the client reconnects (retrying
 * ECONNREFUSED after a short sleep) and re-sends the unacknowledged operation (at-least-once).
 * --pace-ms sleeps that long after each operation (so that a kill can land mid-workload; see the README). The operation list is a pure
 * function of the client id: never of time or randomness.
 *
 * The sequence counter is the first 8 bytes of <tmp>/ds-seq (mapped shared): one atomic increment at invoke and one at
 * completion. At exit the client writes <tmp>/ds-hist/<node>, one write(2) per record, and prints its result line.
 * Nothing here has been run under Hermit.
 */
#include "ds_common.h"

#include <sys/mman.h>

#define N0_OPS 40
#define C0_OPS 60
#define MAX_SERVERS 3
#define MAX_RESENDS 50
#define RECONNECT_TRIES 400
#define MAX_EVENTS (2 * C0_OPS)

typedef struct {
    int is_put;
    int key;
    uint64_t value;
    int server;
} op_t;

typedef struct {
    unsigned long long seq;
    int is_return;
    int is_put;
    int key;
    uint64_t value;
    char result[32];
} event_t;

static op_t op_at(int client_id, int index, int reconnect, int servers) {
    op_t op;
    op.is_put = reconnect ? 1 : (index % 4 != 3);
    op.key = (index * (2 * client_id + 1) + client_id) % DS_KEYS;
    op.server = reconnect ? 0 : (index + client_id) % servers;
    op.value = op.is_put ? (uint64_t)(client_id * 1000 + index) : 0;
    return op;
}

typedef enum { XR_OK, XR_EOF, XR_RESET, XR_TIMEOUT, XR_BAD } xres_t;

static xres_t exchange(int fd, const char *req, char *reply, size_t cap) {
    if (ds_send_all(fd, req, strlen(req)) < 0) return errno == EPIPE || errno == ECONNRESET ? XR_RESET : XR_BAD;
    size_t len = 0;
    for (;;) {
        struct pollfd p = {fd, POLLIN, 0};
        int n = poll(&p, 1, DS_REPLY_WAIT_MS);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return XR_TIMEOUT;
        ssize_t r = recv(fd, reply + len, cap - 1 - len, 0);
        if (r == 0) return XR_EOF;
        if (r < 0) return errno == ECONNRESET ? XR_RESET : XR_BAD;
        len += (size_t)r;
        char *nl = memchr(reply, '\n', len);
        if (nl != NULL) {
            *nl = '\0';
            return XR_OK;
        }
        if (len >= cap - 1) return XR_BAD;
    }
}

static int reply_is_valid(const op_t *op, const char *reply) {
    if (op->is_put) return strcmp(reply, "OK") == 0;
    return strcmp(reply, "NIL") == 0 || strncmp(reply, "VAL ", 4) == 0;
}

static void print_ops(int client_id, int reconnect) {
    int count = reconnect ? C0_OPS : N0_OPS;
    for (int i = 0; i < count; i++) {
        op_t op = op_at(client_id, i, reconnect, MAX_SERVERS);
        if (op.is_put) ds_emit("%d put k%d %llu s%d\n", i, op.key, (unsigned long long)op.value, op.server);
        else ds_emit("%d get k%d - s%d\n", i, op.key, op.server);
    }
}

static int parse_ports(const char *list, int *ports) {
    int n = 0;
    char buf[64];
    snprintf(buf, sizeof buf, "%s", list);
    for (char *tok = strtok(buf, ","); tok != NULL; tok = strtok(NULL, ",")) {
        if (n >= MAX_SERVERS) ds_die("too many servers");
        ports[n++] = ds_parse_port(tok);
    }
    return n;
}

static uint64_t *map_sequence(const char *tmp) {
    char path[512];
    ds_fmt_or_die(path, sizeof path, "%s/ds-seq", tmp);
    int fd = open(path, O_RDWR);
    if (fd < 0) ds_die("cannot open the sequence file");
    void *m = mmap(NULL, DS_SEQ_FILE_BYTES, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (m == MAP_FAILED) ds_die("cannot map the sequence file");
    return (uint64_t *)m;
}

/* Format one record, write it with a single write(2), fold it into the digest. */
static void write_record(int fd, const char *client, const event_t *e, uint64_t *digest) {
    char value[32], line[160];
    if (e->is_put) snprintf(value, sizeof value, "%llu", (unsigned long long)e->value);
    else snprintf(value, sizeof value, "-");
    int n = snprintf(line, sizeof line, "%llu %s %s %s k%d %s %s\n", e->seq, client, e->is_return ? "ret" : "inv",
                     e->is_put ? "put" : "get", e->key, value, e->is_return ? e->result : "-");
    ssize_t w = write(fd, line, (size_t)n);
    (void)w;
    *digest = ds_fnv(*digest, line, (size_t)n);
}

int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    const char *node = ds_opt(argc, argv, "--node=");
    if (node == NULL || node[0] != 'c' || node[1] < '1' || node[1] > '9' || node[2] != '\0') ds_die("usage: dscli --node=c1 ...");
    int client_id = node[1] - '0';
    int reconnect = ds_flag(argc, argv, "--reconnect");
    if (ds_flag(argc, argv, "--print-ops")) {
        print_ops(client_id, reconnect);
        return 0;
    }
    const char *servers_arg = ds_opt(argc, argv, "--servers=");
    const char *tmp = ds_opt(argc, argv, "--tmp=");
    if (servers_arg == NULL || tmp == NULL) ds_die("usage: dscli --node=c1 --servers=P1,P2,P3 --tmp=DIR");
    int per_op_conn = ds_flag(argc, argv, "--per-op-conn");
    const char *pace_arg = ds_opt(argc, argv, "--pace-ms=");
    unsigned pace_ms = pace_arg != NULL ? (unsigned)ds_parse_u64(pace_arg, "bad --pace-ms") : 0;
    int ports[MAX_SERVERS];
    int nservers = parse_ports(servers_arg, ports);
    if (nservers < 1) ds_die("no servers");

    int op_count = reconnect ? C0_OPS : N0_OPS;
    uint64_t *seq = map_sequence(tmp);
    /* Test hook: take one sequence number that no record carries, so that the launcher's numbering check can fail. */
    if (ds_flag(argc, argv, "--test-skip-seq")) __atomic_add_fetch(seq, 1, __ATOMIC_SEQ_CST);
    int fds[MAX_SERVERS];
    unsigned refused = 0, eof = 0, reset = 0, resent = 0, ok = 0, fail = 0;
    for (int s = 0; s < nservers; s++) {
        fds[s] = per_op_conn ? -1 : ds_connect_retry(ports[s], reconnect ? RECONNECT_TRIES : DS_CONNECT_TRIES, &refused);
    }

    static event_t events[MAX_EVENTS];
    int nevents = 0;
    for (int i = 0; i < op_count; i++) {
        op_t op = op_at(client_id, i, reconnect, nservers);
        event_t inv = {0};
        inv.seq = __atomic_add_fetch(seq, 1, __ATOMIC_SEQ_CST);
        inv.is_put = op.is_put;
        inv.key = op.key;
        inv.value = op.value;
        events[nevents++] = inv;

        char req[64], reply[128];
        if (op.is_put) snprintf(req, sizeof req, "PUT k%d %llu\n", op.key, (unsigned long long)op.value);
        else snprintf(req, sizeof req, "GET k%d\n", op.key);

        int valid = 0;
        reply[0] = '\0';
        for (int attempt = 0; attempt <= MAX_RESENDS; attempt++) {
            if (per_op_conn) fds[op.server] = ds_connect_retry(ports[op.server], DS_CONNECT_TRIES, &refused);
            else if (fds[op.server] < 0 && reconnect) fds[op.server] = ds_connect_retry(ports[op.server], RECONNECT_TRIES, &refused);
            if (fds[op.server] < 0) break;
            xres_t x = exchange(fds[op.server], req, reply, sizeof reply);
            if (per_op_conn || x != XR_OK) {
                close(fds[op.server]);
                fds[op.server] = -1;
            }
            if (x == XR_OK) {
                valid = reply_is_valid(&op, reply);
                break;
            }
            if (x == XR_EOF) eof++;
            if (x == XR_RESET) reset++;
            if (!reconnect || (x != XR_EOF && x != XR_RESET)) break;
            resent++;
        }
        if (valid) ok++;
        else fail++;

        event_t ret = inv;
        ret.seq = __atomic_add_fetch(seq, 1, __ATOMIC_SEQ_CST);
        ret.is_return = 1;
        if (!valid) snprintf(ret.result, sizeof ret.result, "FAIL");
        else if (op.is_put) snprintf(ret.result, sizeof ret.result, "OK");
        else snprintf(ret.result, sizeof ret.result, "%s", reply[0] == 'N' ? "NIL" : reply + 4);
        events[nevents++] = ret;
        if (pace_ms > 0) ds_sleep_ms(pace_ms);
    }
    for (int s = 0; s < nservers; s++)
        if (fds[s] >= 0) close(fds[s]);

    char path[512];
    ds_fmt_or_die(path, sizeof path, "%s/ds-hist/%s", tmp, node);
    int hfd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (hfd < 0) ds_die("cannot write the history file");
    uint64_t digest = DS_FNV_OFFSET;
    for (int e = 0; e < nevents; e++) write_record(hfd, node, &events[e], &digest);
    close(hfd);

    if (reconnect) {
        ds_emit("DSC0 v1 role=cli ops=%d ok=%u eof=%u reset=%u refused=%u resent=%u hist=%016llx\n", op_count, ok, eof,
                reset, refused, resent, (unsigned long long)digest);
    } else {
        ds_emit("DSN0 v1 role=cli node=%s ops=%d ok=%u fail=%u refused=%u hist=%016llx\n", node, op_count, ok, fail,
                refused, (unsigned long long)digest);
    }
    return 0;
}
