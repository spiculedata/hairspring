/* dsnode: the crash experiment node. An append-only record log, recovery, and a report region that survives SIGKILL.
 *
 *   HAIRSPRING_PROC=<node>.i<k> dsnode --node=n1 --port=P --tmp=DIR [--helper] [--threads=2] [--pause]
 *   HAIRSPRING_PROC=<node>.i<k> dsnode --node=n1 --tmp=DIR --recover-only
 *
 * The variable only names the incarnation (no SDK is linked). The node:
 *   1. opens or creates <tmp>/ds-data/<node>.log and recovers it: counts records with a valid magic (recovered),
 *      counts bad full records and a partial tail (torn), truncates a partial tail, rebuilds the key table, and prints
 *      "DSC0 v1 role=node tag=<tag> recovered=<u> torn=<u>";
 *   2. creates <tmp>/hairspring-region/<tag> (4,096 bytes, mapped shared): the header, then three counters
 *      (started, done, acked) updated with release atomics and no syscall;
 *   3. listens and serves PUT k<d> <u64> (append one 32-byte record with ONE write(2), reply OK), GET, QUIT.
 *
 * --helper   forks a helper child (same process group) that loops on a 5 ms sleep, for the process-group kill test
 * --threads=2  a second thread appends a heartbeat record every 5 ms through the same lock and counters
 * --pause    after start-up, block in pause() (for the write-then-kill diagnostic)
 * --recover-only  recover, print the start line, exit (native tests)
 * Nothing here has been run under Hermit.
 */
#include "ds_common.h"

#include <pthread.h>
#include <sys/mman.h>

#define RECORD_BYTES 32
#define RECORD_MAGIC 0x44534331u
#define HEARTBEAT_CLIENT 0xFFFFFFFFu
#define REGION_BYTES 4096
#define REGION_HEADER_BYTES 64
#define HEARTBEAT_MS 5
#define IDLE_WAIT_MS 50
#define MAX_IDLE_WAITS 2400
#define MAX_CONNS 32
#define HELPER_MAX_LOOPS 24000
#define HELPER_REAP_POLLS 1000

typedef struct {
    uint32_t magic, incarnation, client, op, key;
    uint64_t value;
    uint32_t reserved;
} __attribute__((packed)) record_t;

_Static_assert(sizeof(record_t) == RECORD_BYTES, "record layout");

static const char *g_tag;
static uint32_t g_incarnation;
static int g_logfd = -1;
static uint64_t *g_ctr; /* started, done, acked */
static ds_table g_table;
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static int g_stop_thread;
static uint32_t g_hb_seq;

static void barrier(void) { __atomic_signal_fence(__ATOMIC_SEQ_CST); }

/* Append one record with one write(2), counted by the region: started, write, done. */
static void append_record(uint32_t client, uint32_t op, uint32_t key, uint64_t value) {
    record_t rec;
    memset(&rec, 0, sizeof rec);
    rec.magic = RECORD_MAGIC;
    rec.incarnation = g_incarnation;
    rec.client = client;
    rec.op = op;
    rec.key = key;
    rec.value = value;
    __atomic_add_fetch(&g_ctr[0], 1, __ATOMIC_RELEASE);
    barrier();
    ssize_t w = write(g_logfd, &rec, sizeof rec);
    barrier();
    (void)w;
    __atomic_add_fetch(&g_ctr[1], 1, __ATOMIC_RELEASE);
}

static void *heartbeat_thread(void *arg) {
    (void)arg;
    while (!__atomic_load_n(&g_stop_thread, __ATOMIC_ACQUIRE)) {
        ds_sleep_ms(HEARTBEAT_MS);
        pthread_mutex_lock(&g_lock);
        append_record(HEARTBEAT_CLIENT, g_hb_seq++, 0xFF, 0);
        __atomic_add_fetch(&g_ctr[2], 1, __ATOMIC_RELEASE);
        pthread_mutex_unlock(&g_lock);
    }
    return NULL;
}

/* Recovery: count, truncate a partial tail, rebuild the table. Prints the start line. */
static void recover(const char *path) {
    g_logfd = open(path, O_RDWR | O_CREAT | O_APPEND, 0644);
    if (g_logfd < 0) ds_die("cannot open the log");
    struct stat st;
    if (fstat(g_logfd, &st) < 0) ds_die("cannot stat the log");
    unsigned recovered = 0, torn = 0;
    off_t full = st.st_size / RECORD_BYTES;
    for (off_t n = 0; n < full; n++) {
        record_t rec;
        if (pread(g_logfd, &rec, sizeof rec, n * RECORD_BYTES) != (ssize_t)sizeof rec) ds_die("cannot read the log");
        if (rec.magic != RECORD_MAGIC) {
            torn++;
            continue;
        }
        recovered++;
        if (rec.client != HEARTBEAT_CLIENT && rec.key < DS_KEYS) {
            g_table.present[rec.key] = 1;
            g_table.value[rec.key] = rec.value;
        }
    }
    if (st.st_size % RECORD_BYTES != 0) {
        torn++;
        if (ftruncate(g_logfd, full * RECORD_BYTES) < 0) ds_die("cannot truncate the torn tail");
    }
    ds_emit("DSC0 v1 role=node tag=%s recovered=%u torn=%u\n", g_tag, recovered, torn);
}

static void create_region(const char *dir) {
    char path[512];
    snprintf(path, sizeof path, "%s/hairspring-region/%s", dir, g_tag);
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0 || ftruncate(fd, REGION_BYTES) < 0) ds_die("cannot create the region");
    unsigned char *m = mmap(NULL, REGION_BYTES, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (m == MAP_FAILED) ds_die("cannot map the region");
    memcpy(m, "HSRGNv1\0", 8);
    uint32_t version = 1, header_bytes = REGION_HEADER_BYTES;
    memcpy(m + 8, &version, 4);
    memcpy(m + 12, &header_bytes, 4);
    snprintf((char *)m + 16, 48, "%s", g_tag);
    __atomic_thread_fence(__ATOMIC_RELEASE);
    g_ctr = (uint64_t *)(m + REGION_HEADER_BYTES);
}

/* Tag "<node>.i<k>": returns k, or dies. */
static uint32_t parse_incarnation(const char *tag) {
    const char *dot = strstr(tag, ".i");
    if (dot == NULL || dot[2] == '\0') ds_die("HAIRSPRING_PROC must look like n1.i0");
    return (uint32_t)ds_parse_u64(dot + 2, "bad incarnation in HAIRSPRING_PROC");
}

static void handle_line(ds_conn *c, const char *line, int *quit) {
    char cmd[8] = "", a[16] = "", b[32] = "";
    int n = sscanf(line, "%7s %15s %31s", cmd, a, b);
    if (n < 1) return;
    if (strcmp(cmd, "PUT") == 0 && n == 3) {
        int k = ds_key_index(a);
        char *end = NULL;
        errno = 0;
        unsigned long long v = strtoull(b, &end, 10);
        if (k < 0 || errno != 0 || end == b || *end != '\0') {
            (void)ds_send_all(c->fd, "ERR\n", 4);
            return;
        }
        pthread_mutex_lock(&g_lock);
        append_record((uint32_t)(v / 1000), (uint32_t)(v % 1000), (uint32_t)k, v);
        g_table.present[k] = 1;
        g_table.value[k] = v;
        (void)ds_send_all(c->fd, "OK\n", 3);
        __atomic_add_fetch(&g_ctr[2], 1, __ATOMIC_RELEASE);
        pthread_mutex_unlock(&g_lock);
    } else if (strcmp(cmd, "GET") == 0 && n == 2) {
        int k = ds_key_index(a);
        char buf[48];
        if (k < 0) snprintf(buf, sizeof buf, "ERR\n");
        else if (g_table.present[k]) snprintf(buf, sizeof buf, "VAL %llu\n", (unsigned long long)g_table.value[k]);
        else snprintf(buf, sizeof buf, "NIL\n");
        (void)ds_send_all(c->fd, buf, strlen(buf));
    } else if (strcmp(cmd, "QUIT") == 0) {
        *quit = 1;
    } else {
        (void)ds_send_all(c->fd, "ERR\n", 4);
    }
}

static void run_helper(pid_t parent) {
    for (int i = 0; i < HELPER_MAX_LOOPS; i++) {
        ds_sleep_ms(5);
        if (getppid() != parent) break;
    }
    _exit(0);
}

static int serve(int port, int use_thread) {
    ds_ev ev;
    ds_conn conns[MAX_CONNS];
    for (int i = 0; i < MAX_CONNS; i++) conns[i].fd = -1;
    if (ds_ev_init(&ev) < 0) ds_die("cannot create the readiness set");
    int lfd = ds_listen(port, 1);
    if (lfd < 0 || ds_ev_add(&ev, lfd) < 0) ds_die("cannot listen");

    pthread_t tid;
    if (use_thread && pthread_create(&tid, NULL, heartbeat_thread, NULL) != 0) ds_die("cannot start the thread");

    int quit = 0;
    unsigned idle = 0;
    int exit_code = 0;
    while (!quit) {
        int ready[DS_EV_MAX];
        int n = ds_ev_wait(&ev, IDLE_WAIT_MS, ready, DS_EV_MAX);
        if (n < 0) {
            exit_code = 1;
            break;
        }
        idle = n == 0 ? idle + 1 : 0;
        if (idle >= MAX_IDLE_WAITS) {
            exit_code = 4;
            break;
        }
        for (int i = 0; i < n; i++) {
            if (ready[i] == lfd) {
                for (;;) {
                    int fd = accept(lfd, NULL, NULL);
                    if (fd < 0) break;
                    ds_nosigpipe(fd);
                    ds_conn *slot = NULL;
                    for (int j = 0; j < MAX_CONNS && slot == NULL; j++)
                        if (conns[j].fd < 0) slot = &conns[j];
                    if (slot == NULL || ds_set_nonblock(fd) < 0 || ds_ev_add(&ev, fd) < 0) {
                        close(fd);
                        continue;
                    }
                    slot->fd = fd;
                    slot->role = 0;
                    slot->len = 0;
                }
                continue;
            }
            ds_conn *c = NULL;
            for (int j = 0; j < MAX_CONNS; j++)
                if (conns[j].fd == ready[i]) c = &conns[j];
            if (c == NULL) continue;
            ssize_t r = recv(c->fd, c->buf + c->len, sizeof c->buf - c->len, 0);
            if (r > 0) {
                c->len += (size_t)r;
                char line[128];
                while (ds_conn_next_line(c, line, sizeof line)) handle_line(c, line, &quit);
            } else if (r == 0 || (errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
                ds_ev_del(&ev, c->fd);
                close(c->fd);
                c->fd = -1;
            }
        }
    }
    if (use_thread) {
        __atomic_store_n(&g_stop_thread, 1, __ATOMIC_RELEASE);
        pthread_join(tid, NULL);
    }
    ds_emit("DSC0 v1 role=node tag=%s exit=%s done=%llu\n", g_tag, exit_code == 0 ? "quit" : "idle",
            (unsigned long long)__atomic_load_n(&g_ctr[1], __ATOMIC_ACQUIRE));
    return exit_code;
}

int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    g_tag = getenv("HAIRSPRING_PROC");
    const char *node = ds_opt(argc, argv, "--node=");
    const char *tmp = ds_opt(argc, argv, "--tmp=");
    const char *port = ds_opt(argc, argv, "--port=");
    int recover_only = ds_flag(argc, argv, "--recover-only");
    if (g_tag == NULL || node == NULL || tmp == NULL || (port == NULL && !recover_only))
        ds_die("usage: HAIRSPRING_PROC=n1.i0 dsnode --node=n1 --port=P --tmp=DIR");
    g_incarnation = parse_incarnation(g_tag);
    const char *threads = ds_opt(argc, argv, "--threads=");
    int use_thread = threads != NULL && ds_parse_u64(threads, "bad --threads") == 2;
    if (threads != NULL && !use_thread) ds_die("--threads must be 2");

    char path[512];
    snprintf(path, sizeof path, "%s/ds-data/%s.log", tmp, node);
    recover(path);
    if (recover_only) return 0;
    create_region(tmp);

    pid_t helper = -1;
    if (ds_flag(argc, argv, "--helper")) {
        pid_t parent = getpid();
        helper = fork();
        if (helper < 0) ds_die("cannot fork the helper");
        if (helper == 0) run_helper(parent);
    }
    if (ds_flag(argc, argv, "--pause")) {
        int lfd = ds_listen(ds_parse_port(port), 1);
        if (lfd < 0) ds_die("cannot listen");
        for (;;) pause();
    }
    int rc = serve(ds_parse_port(port), use_thread);
    if (helper > 0) {
        kill(helper, SIGKILL);
        for (int i = 0; i < HELPER_REAP_POLLS; i++) {
            if (waitpid(helper, NULL, WNOHANG) != 0) break;
            ds_sleep_ms(1);
        }
    }
    return rc;
}
