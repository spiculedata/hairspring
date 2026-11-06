/* Shared helpers for the DS experiment fixtures (dsl, dskv, dscli, dsnode). Header only, libc only.
 *
 * Rules every fixture follows (checked by tests/check_rules.py and by the native tests):
 *   - finite timeouts only, no name lookups (numeric 127.0.0.1 only), no randomness;
 *   - every output line is written with one write(2);
 *   - no pid, port, path or clock value in an output line.
 * Nothing here has been run under Hermit. These are experiment fixtures, not product code.
 */
#ifndef DS_COMMON_H
#define DS_COMMON_H

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#ifdef __linux__
#include <sys/epoll.h>
#endif

#define DS_KEYS 8
#define DS_LINE_MAX 4096
#define DS_FNV_OFFSET 0xcbf29ce484222325ULL
#define DS_FNV_PRIME 0x100000001b3ULL
#define DS_CONNECT_TRIES 200
#define DS_CONNECT_RETRY_MS 5
#define DS_REPLY_WAIT_MS 10000
#define DS_IDLE_LIMIT_MS 120000ULL /* orphan guard: this long without a request */
#define DS_MAX_IDLE_WAITS 2400u    /* orphan guard: this many 50 ms readiness waits with nothing ready */

#ifdef MSG_NOSIGNAL
#define DS_SEND_FLAGS MSG_NOSIGNAL
#else
#define DS_SEND_FLAGS 0
#endif

/* ---- output ---- */

static inline void ds_die(const char *msg);

/* One formatted line to stdout with a single write(2). Returns the length. */
static inline int ds_emit(const char *fmt, ...) {
    char buf[DS_LINE_MAX + 1];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (n < 0) return -1;
    if (n > DS_LINE_MAX) ds_die("output line too long"); /* at most 4,096 bytes with the newline; never written truncated */
    ssize_t w = write(1, buf, (size_t)n);
    return w < 0 ? -1 : n;
}

static inline void ds_die(const char *msg) {
    char buf[256];
    int n = snprintf(buf, sizeof buf, "error: %s\n", msg);
    if (n > 0) {
        ssize_t w = write(2, buf, (size_t)n);
        (void)w;
    }
    _exit(2);
}

/* Format into buf; a result that does not fit is an error (rc 2), never a silently shortened path or argument. */
static inline void ds_fmt_or_die(char *buf, size_t cap, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, cap, fmt, ap);
    va_end(ap);
    if (n < 0 || (size_t)n >= cap) ds_die("a path or argument is too long");
}

/* A node, client or tag name used in paths: 1 to 16 letters and digits. */
static inline int ds_valid_name(const char *s) {
    size_t n = strlen(s);
    if (n == 0 || n > 16) return 0;
    for (size_t i = 0; i < n; i++)
        if (!((s[i] >= 'a' && s[i] <= 'z') || (s[i] >= 'A' && s[i] <= 'Z') || (s[i] >= '0' && s[i] <= '9'))) return 0;
    return 1;
}

/* HAIRSPRING_PROC must be exactly "<node>.i<digits>" for the given node name. */
static inline int ds_valid_tag(const char *tag, const char *node) {
    size_t n = strlen(node);
    if (!ds_valid_name(node) || strncmp(tag, node, n) != 0 || tag[n] != '.' || tag[n + 1] != 'i') return 0;
    const char *d = tag + n + 2;
    size_t len = strlen(d);
    if (len == 0 || len > 9) return 0;
    for (size_t i = 0; i < len; i++)
        if (d[i] < '0' || d[i] > '9') return 0;
    return 1;
}

/* ---- digests and the splitmix finaliser ---- */

static inline uint64_t ds_fnv(uint64_t h, const void *data, size_t len) {
    const unsigned char *p = (const unsigned char *)data;
    for (size_t i = 0; i < len; i++) h = (h ^ p[i]) * DS_FNV_PRIME;
    return h;
}

static inline uint64_t ds_mix(uint64_t z) {
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

/* ---- arguments ---- */

/* The value of "--name=value", or NULL. */
static inline const char *ds_opt(int argc, char **argv, const char *prefix) {
    size_t n = strlen(prefix);
    for (int i = 1; i < argc; i++)
        if (strncmp(argv[i], prefix, n) == 0) return argv[i] + n;
    return NULL;
}

static inline int ds_flag(int argc, char **argv, const char *name) {
    for (int i = 1; i < argc; i++)
        if (strcmp(argv[i], name) == 0) return 1;
    return 0;
}

static inline uint64_t ds_parse_u64(const char *s, const char *what) {
    char *end = NULL;
    errno = 0;
    unsigned long long v = strtoull(s, &end, 10);
    if (errno != 0 || end == s || *end != '\0') ds_die(what);
    return (uint64_t)v;
}

static inline int ds_parse_port(const char *s) {
    uint64_t v = ds_parse_u64(s, "bad port");
    if (v == 0 || v > 65535) ds_die("bad port");
    return (int)v;
}

/* ---- time ---- */

static inline void ds_sleep_ms(unsigned ms) {
    struct timespec ts = {(time_t)(ms / 1000), (long)(ms % 1000) * 1000000L};
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {
    }
}

static inline uint64_t ds_mono_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000u + (uint64_t)ts.tv_nsec / 1000000u;
}

/* ---- sockets (numeric loopback only) ---- */

static inline void ds_addr(struct sockaddr_in *a, int port) {
    memset(a, 0, sizeof *a);
    a->sin_family = AF_INET;
    a->sin_port = htons((uint16_t)port);
    a->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
}

static inline int ds_set_nonblock(int fd) {
    int fl = fcntl(fd, F_GETFL, 0);
    return fl < 0 ? -1 : fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}

static inline void ds_nosigpipe(int fd) {
#ifdef SO_NOSIGPIPE
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof one);
#else
    (void)fd;
#endif
}

/* A listening socket on 127.0.0.1:port, or -1. */
static inline int ds_listen(int port, int nonblock) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a;
    ds_addr(&a, port);
    if (bind(fd, (struct sockaddr *)&a, sizeof a) < 0 || listen(fd, 16) < 0) {
        close(fd);
        return -1;
    }
    if (nonblock && ds_set_nonblock(fd) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

/* Connect to 127.0.0.1:port, retrying ECONNREFUSED after a short sleep. *refused counts the refusals. -1 on failure. */
static inline int ds_connect_retry(int port, int max_tries, unsigned *refused) {
    for (int attempt = 0; attempt < max_tries; attempt++) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        if (fd < 0) return -1;
        ds_nosigpipe(fd);
        struct sockaddr_in a;
        ds_addr(&a, port);
        if (connect(fd, (struct sockaddr *)&a, sizeof a) == 0) return fd;
        int err = errno;
        close(fd);
        if (err != ECONNREFUSED && err != EINTR) return -1;
        if (refused != NULL) (*refused)++;
        ds_sleep_ms(DS_CONNECT_RETRY_MS);
    }
    return -1;
}

static inline int ds_send_all(int fd, const char *data, size_t len) {
    size_t off = 0;
    while (off < len) {
        ssize_t n = send(fd, data + off, len - off, DS_SEND_FLAGS);
        if (n < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        off += (size_t)n;
    }
    return 0;
}

/* ---- the key table ---- */

typedef struct {
    int present[DS_KEYS];
    uint64_t value[DS_KEYS];
} ds_table;

/* "k<d>" with d in 0..DS_KEYS-1, or -1. */
static inline int ds_key_index(const char *s) {
    if (s[0] != 'k' || s[1] < '0' || s[1] >= '0' + DS_KEYS || s[2] != '\0') return -1;
    return s[1] - '0';
}

/* FNV-1a 64 of the text "k<d>=<value>;" for each present key in index order. */
static inline uint64_t ds_table_digest(const ds_table *t) {
    uint64_t h = DS_FNV_OFFSET;
    for (int k = 0; k < DS_KEYS; k++) {
        if (!t->present[k]) continue;
        char buf[48];
        int n = snprintf(buf, sizeof buf, "k%d=%llu;", k, (unsigned long long)t->value[k]);
        h = ds_fnv(h, buf, (size_t)n);
    }
    return h;
}

/* ---- orphan guards (pure, so that tests can reach them without waiting 120 s) ---- */

/* True when more than limit_ms have passed since the last request. */
static inline int ds_idle_expired(uint64_t now_ms, uint64_t last_ms, uint64_t limit_ms) {
    return now_ms > last_ms && now_ms - last_ms > limit_ms;
}

static inline int ds_idle_waits_expired(unsigned idle_waits) { return idle_waits >= DS_MAX_IDLE_WAITS; }

/* Only a real request counts as activity: heartbeats and HELLO do not. */
static inline int ds_counts_as_request(const char *cmd) {
    return strcmp(cmd, "PUT") == 0 || strcmp(cmd, "GET") == 0 || strcmp(cmd, "REPL") == 0 || strcmp(cmd, "QUIT") == 0;
}

/* ---- the readiness loop: epoll on Linux, poll elsewhere; finite timeouts only ---- */

#define DS_EV_MAX 64

typedef struct {
    int count;
    int fds[DS_EV_MAX];
#ifdef __linux__
    int ep;
#endif
} ds_ev;

static inline int ds_ev_init(ds_ev *ev) {
    ev->count = 0;
#ifdef __linux__
    ev->ep = epoll_create1(0);
    return ev->ep < 0 ? -1 : 0;
#else
    return 0;
#endif
}

static inline int ds_ev_add(ds_ev *ev, int fd) {
    if (ev->count >= DS_EV_MAX) return -1;
#ifdef __linux__
    struct epoll_event e;
    memset(&e, 0, sizeof e);
    e.events = EPOLLIN;
    e.data.fd = fd;
    if (epoll_ctl(ev->ep, EPOLL_CTL_ADD, fd, &e) < 0) return -1;
#endif
    ev->fds[ev->count++] = fd;
    return 0;
}

static inline void ds_ev_del(ds_ev *ev, int fd) {
#ifdef __linux__
    epoll_ctl(ev->ep, EPOLL_CTL_DEL, fd, NULL);
#endif
    for (int i = 0; i < ev->count; i++) {
        if (ev->fds[i] == fd) {
            ev->fds[i] = ev->fds[--ev->count];
            return;
        }
    }
}

/* Wait at most timeout_ms (never negative: a negative value is raised to 0). Fills ready[] and returns the count. */
static inline int ds_ev_wait(ds_ev *ev, int timeout_ms, int *ready, int max_ready) {
    if (timeout_ms < 0) timeout_ms = 0;
#ifdef __linux__
    struct epoll_event out[DS_EV_MAX];
    int n = epoll_wait(ev->ep, out, max_ready < DS_EV_MAX ? max_ready : DS_EV_MAX, timeout_ms);
    if (n < 0) return errno == EINTR ? 0 : -1;
    for (int i = 0; i < n; i++) ready[i] = out[i].data.fd;
    return n;
#else
    struct pollfd p[DS_EV_MAX];
    for (int i = 0; i < ev->count; i++) {
        p[i].fd = ev->fds[i];
        p[i].events = POLLIN;
        p[i].revents = 0;
    }
    int n = poll(p, (nfds_t)ev->count, timeout_ms);
    if (n < 0) return errno == EINTR ? 0 : -1;
    int out_n = 0;
    for (int i = 0; i < ev->count && out_n < max_ready; i++)
        if (p[i].revents != 0) ready[out_n++] = p[i].fd;
    return out_n;
#endif
}

/* ---- line buffers on a connection ---- */

#define DS_CONN_BUF 512

typedef struct {
    int fd;
    int role; /* 0 client or unknown, 1 predecessor (sent HELLO), 2 successor (our outgoing link) */
    size_t len;
    char buf[DS_CONN_BUF];
} ds_conn;

/* Extract the next complete line (without the newline) into line. Returns 1 if one was produced. */
static inline int ds_conn_next_line(ds_conn *c, char *line, size_t cap) {
    char *nl = memchr(c->buf, '\n', c->len);
    if (nl == NULL) {
        if (c->len == sizeof c->buf) c->len = 0; /* an over-long line is dropped */
        return 0;
    }
    size_t n = (size_t)(nl - c->buf);
    size_t copy = n < cap - 1 ? n : cap - 1;
    memcpy(line, c->buf, copy);
    line[copy] = '\0';
    memmove(c->buf, nl + 1, c->len - n - 1);
    c->len -= n + 1;
    return 1;
}

/* ---- the shared sequence counter (first 8 bytes of the mapped sequence file) ---- */

#define DS_SEQ_FILE_BYTES 4096

#endif
