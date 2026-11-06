/* dskv: the loopback experiment key-value server. One process of a three-node ring.
 *
 *   dskv --mode=epoll|timeout|block --node=n1 --port=P --peer-port=Q [--hb-thread] [--tmp=DIR]
 *
 * Line protocol (one request per line, one reply per line):
 *   PUT k<d> <u64>   apply locally, send "REPL k<d> <u64>" to the ring successor (no wait), reply "OK"
 *   GET k<d>         reply "VAL <u64>" or "NIL"
 *   REPL k<d> <u64>  apply a replicated write (no reply)
 *   HELLO <node>     sent once on a new link to the successor; marks the link's far end as our predecessor
 *   HB               heartbeat (timeout mode)
 *   QUIT             drain, print the result line, exit 0
 *
 * epoll    non-blocking sockets, readiness waits with finite timeouts, ring replication over persistent links
 * timeout  as epoll, plus a heartbeat every 20 ms to both peers and "suspect" after 60 ms of silence; every loop reads
 *          the monotonic and real clock and folds the readings into a digest (the clock field)
 * block    logically blocking accept/recv/send, one connection at a time, no replication. accept and recv have no
 *          bound by design; the launcher bounds the process by QUIT and, failing that, by SIGKILL.
 *
 * Test hooks (used only by the native tests): --test-exit-early, --test-ignore-quit.
 * Nothing here has been run under Hermit.
 */
#include "ds_common.h"

#include <pthread.h>

#define MAX_CONNS 32
#define HB_INTERVAL_MS 20
#define SUSPECT_MS 60
#define IDLE_WAIT_MS 50
#define MAX_IDLE_WAITS 2400 /* orphan guard: about 120 s of silence in epoll mode */
#define IDLE_LIMIT_MS 120000 /* orphan guard in timeout mode: no request for this long (heartbeats do not count) */

typedef enum { MODE_EPOLL, MODE_TIMEOUT, MODE_BLOCK } ds_mode;

typedef struct {
    const char *node;
    const char *mode_name;
    ds_mode mode;
    int port;
    int peer_port;
    int use_hb_thread;
    int test_ignore_quit;
    ds_table table;
    uint64_t applied, repl_in, repl_out;
    unsigned refused;
    unsigned hb;      /* atomic: the main loop and the heartbeat thread both count */
    unsigned suspect; /* transitions into suspicion */
    int succ_fd, pred_fd; /* atomic ints; -1 when unknown */
    int quit;
    int stop_thread; /* atomic */
    uint64_t thread_clock;
    uint64_t last_hb_succ, last_hb_pred;
    uint64_t last_activity; /* timeout mode: last PUT, GET, REPL or QUIT */
    int suspect_succ, suspect_pred;
    ds_conn conns[MAX_CONNS];
} server_t;

static server_t S;

static int replicates(void) { return S.mode != MODE_BLOCK; }

static void send_text(int fd, const char *s) {
    if (fd >= 0) (void)ds_send_all(fd, s, strlen(s));
}

static uint64_t fold_clock(uint64_t h) {
    struct timespec mono, real;
    clock_gettime(CLOCK_MONOTONIC, &mono);
    clock_gettime(CLOCK_REALTIME, &real);
    h = ds_fnv(h, &mono, sizeof mono);
    return ds_fnv(h, &real, sizeof real);
}

static void send_heartbeats(void) {
    int succ = __atomic_load_n(&S.succ_fd, __ATOMIC_ACQUIRE);
    int pred = __atomic_load_n(&S.pred_fd, __ATOMIC_ACQUIRE);
    if (succ >= 0) {
        send_text(succ, "HB\n");
        __atomic_add_fetch(&S.hb, 1, __ATOMIC_RELAXED);
    }
    if (pred >= 0) {
        send_text(pred, "HB\n");
        __atomic_add_fetch(&S.hb, 1, __ATOMIC_RELAXED);
    }
}

static void *heartbeat_thread(void *arg) {
    (void)arg;
    uint64_t clock = DS_FNV_OFFSET;
    while (!__atomic_load_n(&S.stop_thread, __ATOMIC_ACQUIRE)) {
        ds_sleep_ms(HB_INTERVAL_MS);
        clock = fold_clock(clock);
        send_heartbeats();
    }
    S.thread_clock = clock;
    return NULL;
}

static void check_one_suspect(int known, uint64_t last, uint64_t now, int *suspected) {
    if (!known) return;
    if (now - last > SUSPECT_MS) {
        if (!*suspected) {
            *suspected = 1;
            S.suspect++;
        }
    } else {
        *suspected = 0;
    }
}

static void check_suspects(uint64_t now) {
    check_one_suspect(__atomic_load_n(&S.succ_fd, __ATOMIC_ACQUIRE) >= 0, S.last_hb_succ, now, &S.suspect_succ);
    check_one_suspect(__atomic_load_n(&S.pred_fd, __ATOMIC_ACQUIRE) >= 0, S.last_hb_pred, now, &S.suspect_pred);
}

static int parse_value(const char *s, uint64_t *out) {
    char *end = NULL;
    errno = 0;
    unsigned long long v = strtoull(s, &end, 10);
    if (errno != 0 || end == s || *end != '\0') return -1;
    *out = (uint64_t)v;
    return 0;
}

static void handle_line(ds_conn *c, const char *line) {
    char cmd[8] = "", a[16] = "", b[32] = "";
    int n = sscanf(line, "%7s %15s %31s", cmd, a, b);
    if (n < 1) return;
    uint64_t now = S.mode == MODE_TIMEOUT ? ds_mono_ms() : 0;
    if (strcmp(cmd, "HB") != 0 && strcmp(cmd, "HELLO") != 0) S.last_activity = now;
    if (strcmp(cmd, "PUT") == 0 || strcmp(cmd, "REPL") == 0) {
        int is_put = cmd[0] == 'P';
        int k = n == 3 ? ds_key_index(a) : -1;
        uint64_t v = 0;
        if (k < 0 || parse_value(b, &v) < 0) {
            if (is_put) send_text(c->fd, "ERR\n");
            return;
        }
        S.table.present[k] = 1;
        S.table.value[k] = v;
        if (!is_put) {
            S.repl_in++;
            return;
        }
        S.applied++;
        int succ = __atomic_load_n(&S.succ_fd, __ATOMIC_ACQUIRE);
        if (replicates() && succ >= 0) {
            char buf[64];
            snprintf(buf, sizeof buf, "REPL k%d %llu\n", k, (unsigned long long)v);
            send_text(succ, buf);
            S.repl_out++;
        }
        send_text(c->fd, "OK\n");
    } else if (strcmp(cmd, "GET") == 0 && n == 2) {
        int k = ds_key_index(a);
        if (k < 0) {
            send_text(c->fd, "ERR\n");
        } else if (S.table.present[k]) {
            char buf[48];
            snprintf(buf, sizeof buf, "VAL %llu\n", (unsigned long long)S.table.value[k]);
            send_text(c->fd, buf);
        } else {
            send_text(c->fd, "NIL\n");
        }
    } else if (strcmp(cmd, "HELLO") == 0) {
        c->role = 1;
        __atomic_store_n(&S.pred_fd, c->fd, __ATOMIC_RELEASE);
        S.last_hb_pred = now;
    } else if (strcmp(cmd, "HB") == 0) {
        if (c->role == 1) S.last_hb_pred = now;
        if (c->role == 2) S.last_hb_succ = now;
    } else if (strcmp(cmd, "QUIT") == 0) {
        if (!S.test_ignore_quit) S.quit = 1;
    } else {
        send_text(c->fd, "ERR\n");
    }
}

static void process_lines(ds_conn *c) {
    char line[128];
    while (ds_conn_next_line(c, line, sizeof line)) handle_line(c, line);
}

static ds_conn *conn_for_fd(int fd) {
    for (int i = 0; i < MAX_CONNS; i++)
        if (S.conns[i].fd == fd) return &S.conns[i];
    return NULL;
}

static ds_conn *conn_add(ds_ev *ev, int fd, int role) {
    ds_conn *c = conn_for_fd(-1);
    if (c == NULL || ds_ev_add(ev, fd) < 0) return NULL;
    c->fd = fd;
    c->role = role;
    c->len = 0;
    return c;
}

/* Links to peers stay open until exit (the heartbeat thread may still hold the descriptor number). */
static void conn_drop(ds_ev *ev, ds_conn *c) {
    ds_ev_del(ev, c->fd);
    if (c->role == 1) __atomic_store_n(&S.pred_fd, -1, __ATOMIC_RELEASE);
    if (c->role == 2) __atomic_store_n(&S.succ_fd, -1, __ATOMIC_RELEASE);
    if (c->role == 0) close(c->fd);
    c->fd = -1;
}

static void read_conn(ds_ev *ev, ds_conn *c) {
    ssize_t r = recv(c->fd, c->buf + c->len, sizeof c->buf - c->len, 0);
    if (r > 0) {
        c->len += (size_t)r;
        process_lines(c);
    } else if (r == 0 || (errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
        conn_drop(ev, c);
    }
}

/* After QUIT: everything already delivered to this process's sockets is consumed before the result is printed. */
static void drain_all(void) {
    for (int i = 0; i < MAX_CONNS; i++) {
        ds_conn *c = &S.conns[i];
        if (c->fd < 0) continue;
        for (;;) {
            ssize_t r = recv(c->fd, c->buf + c->len, sizeof c->buf - c->len, MSG_DONTWAIT);
            if (r <= 0) break;
            c->len += (size_t)r;
            process_lines(c);
        }
    }
}

static void accept_ready(ds_ev *ev, int lfd) {
    for (;;) {
        int fd = accept(lfd, NULL, NULL);
        if (fd < 0) return;
        ds_nosigpipe(fd);
        if (ds_set_nonblock(fd) < 0 || conn_add(ev, fd, 0) == NULL) close(fd);
    }
}

static int connect_successor(ds_ev *ev) {
    int fd = ds_connect_retry(S.peer_port, DS_CONNECT_TRIES, &S.refused);
    if (fd < 0) return -1;
    char hello[32];
    snprintf(hello, sizeof hello, "HELLO %s\n", S.node);
    if (ds_send_all(fd, hello, strlen(hello)) < 0 || ds_set_nonblock(fd) < 0) return -1;
    ds_conn *c = conn_add(ev, fd, 2);
    if (c == NULL) return -1;
    S.last_hb_succ = S.mode == MODE_TIMEOUT ? ds_mono_ms() : 0;
    __atomic_store_n(&S.succ_fd, fd, __ATOMIC_RELEASE);
    return 0;
}

static void print_result(uint64_t clock) {
    ds_emit("DSN0 v1 role=srv node=%s mode=%s applied=%llu repl_in=%llu repl_out=%llu refused=%u hb=%u suspect=%u "
            "clock=%016llx state=%016llx\n",
            S.node, S.mode_name, (unsigned long long)S.applied, (unsigned long long)S.repl_in,
            (unsigned long long)S.repl_out, S.refused, __atomic_load_n(&S.hb, __ATOMIC_RELAXED), S.suspect,
            (unsigned long long)clock, (unsigned long long)ds_table_digest(&S.table));
}

static int run_block(void) {
    int lfd = ds_listen(S.port, 0);
    if (lfd < 0) ds_die("cannot listen");
    while (!S.quit) {
        int cfd = accept(lfd, NULL, NULL);
        if (cfd < 0) {
            if (errno == EINTR) continue;
            return 1;
        }
        ds_conn c;
        memset(&c, 0, sizeof c);
        c.fd = cfd;
        while (!S.quit) {
            ssize_t r = recv(cfd, c.buf + c.len, sizeof c.buf - c.len, 0);
            if (r <= 0) break;
            c.len += (size_t)r;
            process_lines(&c);
        }
        close(cfd);
    }
    close(lfd);
    print_result(0);
    return 0;
}

static int run_event_loop(void) {
    ds_ev ev;
    if (ds_ev_init(&ev) < 0) ds_die("cannot create the readiness set");
    int lfd = ds_listen(S.port, 1);
    if (lfd < 0 || ds_ev_add(&ev, lfd) < 0) ds_die("cannot listen");
    if (connect_successor(&ev) < 0) ds_die("cannot reach the successor");

    pthread_t hb_tid;
    int thread_started = 0;
    if (S.mode == MODE_TIMEOUT && S.use_hb_thread) {
        if (pthread_create(&hb_tid, NULL, heartbeat_thread, NULL) != 0) ds_die("cannot start the heartbeat thread");
        thread_started = 1;
    }

    S.last_activity = S.mode == MODE_TIMEOUT ? ds_mono_ms() : 0;
    uint64_t clock = DS_FNV_OFFSET;
    uint64_t next_hb = S.mode == MODE_TIMEOUT ? ds_mono_ms() + HB_INTERVAL_MS : 0;
    unsigned idle_waits = 0;
    int exit_code = 0;
    while (!S.quit) {
        int timeout = IDLE_WAIT_MS;
        if (S.mode == MODE_TIMEOUT) {
            uint64_t now = ds_mono_ms();
            clock = fold_clock(clock);
            if (!S.use_hb_thread && now >= next_hb) {
                send_heartbeats();
                next_hb = now + HB_INTERVAL_MS;
            }
            check_suspects(now);
            if (now - S.last_activity > IDLE_LIMIT_MS) {
                exit_code = 4;
                break;
            }
            timeout = S.use_hb_thread ? HB_INTERVAL_MS : (int)(next_hb > now ? next_hb - now : 1);
            if (timeout < 1) timeout = 1;
            if (timeout > IDLE_WAIT_MS) timeout = IDLE_WAIT_MS;
        }
        int ready[DS_EV_MAX];
        int n = ds_ev_wait(&ev, timeout, ready, DS_EV_MAX);
        if (n < 0) {
            exit_code = 1;
            break;
        }
        if (S.mode != MODE_TIMEOUT) {
            idle_waits = n == 0 ? idle_waits + 1 : 0;
            if (idle_waits >= MAX_IDLE_WAITS) {
                exit_code = 4;
                break;
            }
        }
        for (int i = 0; i < n; i++) {
            if (ready[i] == lfd) {
                accept_ready(&ev, lfd);
                continue;
            }
            ds_conn *c = conn_for_fd(ready[i]);
            if (c != NULL) read_conn(&ev, c);
        }
    }
    if (thread_started) {
        __atomic_store_n(&S.stop_thread, 1, __ATOMIC_RELEASE);
        pthread_join(hb_tid, NULL);
        clock = ds_mix(clock ^ ds_mix(S.thread_clock));
    }
    drain_all();
    print_result(S.mode == MODE_TIMEOUT ? clock : 0);
    return exit_code;
}

int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    memset(&S, 0, sizeof S);
    for (int i = 0; i < MAX_CONNS; i++) S.conns[i].fd = -1;
    S.succ_fd = S.pred_fd = -1;

    const char *mode = ds_opt(argc, argv, "--mode=");
    const char *node = ds_opt(argc, argv, "--node=");
    const char *port = ds_opt(argc, argv, "--port=");
    const char *peer = ds_opt(argc, argv, "--peer-port=");
    if (mode == NULL || node == NULL || port == NULL) ds_die("usage: dskv --mode=epoll|timeout|block --node=n1 --port=P --peer-port=Q");
    S.mode_name = mode;
    if (strcmp(mode, "epoll") == 0) S.mode = MODE_EPOLL;
    else if (strcmp(mode, "timeout") == 0) S.mode = MODE_TIMEOUT;
    else if (strcmp(mode, "block") == 0) S.mode = MODE_BLOCK;
    else ds_die("unknown mode");
    if (!ds_valid_name(node)) ds_die("bad --node");
    S.node = node;
    S.port = ds_parse_port(port);
    if (S.mode != MODE_BLOCK) {
        if (peer == NULL) ds_die("--peer-port is required outside block mode");
        S.peer_port = ds_parse_port(peer);
    }
    S.use_hb_thread = ds_flag(argc, argv, "--hb-thread");
    S.test_ignore_quit = ds_flag(argc, argv, "--test-ignore-quit");
    if (S.use_hb_thread && S.mode != MODE_TIMEOUT) ds_die("--hb-thread needs --mode=timeout");

    if (S.mode == MODE_BLOCK) return run_block();
    if (ds_flag(argc, argv, "--test-exit-early")) {
        /* Test hook: listen so that the ring can form, then leave before serving anything. */
        int lfd = ds_listen(S.port, 1);
        if (lfd < 0) ds_die("cannot listen");
        _exit(1);
    }
    return run_event_loop();
}
