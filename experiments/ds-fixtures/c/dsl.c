/* dsl: the DS launcher. Starts the processes of one trial, drives them to completion, checks the merged history and
 * prints one result line. Single-threaded, installs no signal handlers, never waits in a blocking call (every wait is a
 * waitpid(WNOHANG) poll with a short sleep, bounded by a poll count), and reaps every child before it exits.
 *
 * loopback experiment trial:
 *   dsl --topo=c3e|c3t|c3t2|c3b|g3|r3 --bin-dir=DIR --tmp=DIR --port-base=7101
 * crash experiment trial (crash mode):
 *   dsl --crash=pid,K|pgid,K --bin-dir=DIR --tmp=DIR --port=7201 [--seed=S] [--node-bin=dsnode|dsgonode]
 *       [--helper] [--threads=2] [--pause] [--raw-write-kill] [--pace-ms=N]
 * Plan only:
 *   dsl --print-plan --seed=S --kills=K
 * Test hooks (native tests only): --server-arg=<node>:<arg> passes one extra argument to one loopback-experiment server
 * or client, for example n3:--test-ignore-quit or c1:--test-skip-seq (repeatable, at most 4); --test-leak-child leaves
 * an untracked child alive for a moment.
 *
 * The crash plan is a pure function of the seed (splitmix64 finaliser keyed by salt and kill index). The seed comes
 * from --seed only. Nothing here has been run under Hermit.
 */
#include "ds_common.h"

#include <sched.h>

#define MAX_PROCS 8
#define MAX_EXTRA 4
#define MAX_KILLS 2
#define CLIENT_POLL_MS 10
#define CLIENT_POLLS 6000 /* 60,000 ms */
#define SERVER_POLL_MS 10
#define SERVER_POLLS 200 /* 2,000 ms */
#define KILL_REAP_POLLS 5000
#define STRAGGLER_REAP_POLLS 1000
#define GROUP_GONE_POLLS 1000
#define N0_OPS 40
#define C0_OPS 60
#define CRASH_SALT 0x6372617368ULL
#define KW_WAIT_MS 50
#define KW_PLANNED_MS 50
#define HELPER_LOOPS 24000
#define HIST_BUF_BYTES (64 * 1024)
#define MAX_RECORDS 512
#define DEFAULT_PACE_MS 10
#define ARG_BYTES 512
#define LEAK_CHILD_MS 1500
#define PRE_KILL_LINE "DSC0 v1 role=sup pre-kill\n"

typedef struct {
    pid_t pid;
    char name[16];
    int alive;
    int exited; /* a status was collected */
    int status;
    char tag[24];          /* the name in an "exited" line */
    int reported;          /* its "exited" line has been written */
    int planned_sent;      /* the launcher sent a planned kill to it after finding it alive */
    int stop_killed;       /* the launcher SIGKILLed it by pid at teardown */
    int expect_clean_exit; /* a normal exit with status 0 is the expected end (client, final incarnation) */
    int stub;              /* test stubs only (DS_STUB_WAITPID) */
    int stub_used;
} proc_t;

typedef struct {
    char *v[24];
    char store[24][ARG_BYTES];
    int n;
} argv_t;

typedef struct {
    char node[8];
    char arg[48];
} extra_t;

static const char *g_bin_dir;
static const char *g_tmp;
static extra_t g_extra[MAX_EXTRA];
static int g_extra_count;
static int g_leak_child;

/* ---- small helpers ---- */

static void av_add(argv_t *a, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int len = vsnprintf(a->store[a->n], sizeof a->store[a->n], fmt, ap);
    va_end(ap);
    if (len < 0 || len >= ARG_BYTES || a->n >= 22) ds_die("a path or argument is too long");
    a->v[a->n] = a->store[a->n];
    a->n++;
    a->v[a->n] = NULL;
}

static void av_init(argv_t *a, const char *program) {
    a->n = 0;
    av_add(a, "%s/%s", g_bin_dir, program);
}

static pid_t spawn(proc_t *p, const char *name, argv_t *a, const char *tag) {
    pid_t pid = fork();
    if (pid < 0) ds_die("fork failed");
    if (pid == 0) {
        setpgid(0, 0);
        if (tag != NULL) {
            setenv("HAIRSPRING_PROC", tag, 1);
            unsetenv("HAIRSPRING_PROC_PID");
        }
        execv(a->v[0], a->v);
        _exit(127);
    }
    setpgid(pid, pid); /* race-free: the child does the same; whichever runs first wins */
    memset(p, 0, sizeof *p);
    p->pid = pid;
    p->alive = 1;
    snprintf(p->name, sizeof p->name, "%s", name);
    return pid;
}

/* The one place the launcher asks whether a tracked child has ended. DS_STUB_WAITPID is a compile-time option used only
 * by the native tests to reach cases that cannot happen natively: 1 = the first node's first answer is an error, 2 =
 * the first node is never reaped. Real builds do not define it. */
static pid_t ds_wait_child(proc_t *p, int *st) {
#ifdef DS_STUB_WAITPID
    if (p->stub) {
#if DS_STUB_WAITPID == 1
        if (!p->stub_used) {
            p->stub_used = 1;
            errno = ECHILD;
            return -1;
        }
#else
        return 0;
#endif
    }
#endif
    return waitpid(p->pid, st, WNOHANG);
}

/* One non-blocking attempt to collect a child. Returns 1 when it is gone. */
static int reap_once(proc_t *p) {
    if (!p->alive) return 1;
    int st = 0;
    pid_t r = ds_wait_child(p, &st);
    if (r == p->pid) {
        p->alive = 0;
        p->exited = 1;
        p->status = st;
        return 1;
    }
    if (r < 0 && errno != EINTR) {
        p->alive = 0;
        return 1;
    }
    return 0;
}

/* Poll until every process is gone or the poll budget is spent. */
static void wait_all(proc_t *ps, int n, int polls, unsigned interval_ms) {
    for (int k = 0; k < polls; k++) {
        int gone = 1;
        for (int i = 0; i < n; i++)
            if (!reap_once(&ps[i])) gone = 0;
        if (gone) return;
        ds_sleep_ms(interval_ms);
    }
}

/* SIGKILL by pid whatever is still alive, count it, and reap it. */
static unsigned kill_stragglers(proc_t *ps, int n) {
    unsigned count = 0;
    for (int i = 0; i < n; i++) {
        if (!ps[i].alive) continue;
        count++;
        ps[i].stop_killed = 1;
        kill(ps[i].pid, SIGKILL);
    }
    if (count > 0) wait_all(ps, n, STRAGGLER_REAP_POLLS, 1);
    return count;
}

static void send_quit(int port) {
    int fd = ds_connect_retry(port, 3, NULL);
    if (fd < 0) return;
    ds_nosigpipe(fd);
    (void)ds_send_all(fd, "QUIT\n", 5);
    close(fd);
}

/* Test hook: a child the launcher does not track, to prove that the live-child check can fail. It exits by itself. */
static void maybe_leak_child(void) {
    if (!g_leak_child) return;
    pid_t pid = fork();
    if (pid == 0) {
        ds_sleep_ms(LEAK_CHILD_MS);
        _exit(0);
    }
}

static int has_live_child(void) {
    int st;
    pid_t r = waitpid(-1, &st, WNOHANG);
    return !(r < 0 && errno == ECHILD);
}

static void path_in_tmp(char *buf, size_t cap, const char *rel) { ds_fmt_or_die(buf, cap, "%s/%s", g_tmp, rel); }

static void make_dir(const char *rel) {
    char path[512];
    path_in_tmp(path, sizeof path, rel);
    if (mkdir(path, 0755) < 0 && errno != EEXIST) ds_die("cannot create a trial directory");
}

static void setup_trial_files(int crash) {
    if (mkdir(g_tmp, 0755) < 0 && errno != EEXIST) ds_die("cannot create the trial directory");
    make_dir("ds-hist");
    if (crash) {
        make_dir("ds-data");
        make_dir("hairspring-region");
    }
    char path[512];
    path_in_tmp(path, sizeof path, "ds-seq");
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0 || ftruncate(fd, DS_SEQ_FILE_BYTES) < 0) ds_die("cannot create the sequence file");
    close(fd);
}

/* ---- merged history ---- */

typedef struct {
    unsigned long long seq;
    const char *line;
    size_t len;
} record_t;

static int cmp_record(const void *a, const void *b) {
    const record_t *x = a, *y = b;
    return x->seq < y->seq ? -1 : x->seq > y->seq;
}

/* Read the client history files, sort the records by seq, check the numbering is exactly 1..expected. */
static int check_history(const char **clients, int n, unsigned expected, uint64_t *digest_out, unsigned *count_out) {
    static char bufs[3][HIST_BUF_BYTES];
    static record_t recs[MAX_RECORDS];
    unsigned count = 0;
    for (int c = 0; c < n && c < 3; c++) {
        char rel[64], path[512];
        snprintf(rel, sizeof rel, "ds-hist/%s", clients[c]);
        path_in_tmp(path, sizeof path, rel);
        int fd = open(path, O_RDONLY);
        if (fd < 0) continue;
        ssize_t len = read(fd, bufs[c], HIST_BUF_BYTES - 1);
        close(fd);
        if (len <= 0) continue;
        bufs[c][len] = '\0';
        for (char *p = bufs[c]; *p != '\0' && count < MAX_RECORDS;) {
            char *nl = strchr(p, '\n');
            if (nl == NULL) break;
            recs[count].seq = strtoull(p, NULL, 10);
            recs[count].line = p;
            recs[count].len = (size_t)(nl - p) + 1;
            count++;
            p = nl + 1;
        }
    }
    qsort(recs, count, sizeof recs[0], cmp_record);
    int ok = count == expected;
    uint64_t digest = DS_FNV_OFFSET;
    for (unsigned i = 0; i < count; i++) {
        if (recs[i].seq != i + 1) ok = 0;
        digest = ds_fnv(digest, recs[i].line, recs[i].len);
    }
    *digest_out = digest;
    *count_out = count;
    return ok;
}

static void format_status(char *out, size_t cap, const proc_t *p) {
    if (p->exited && WIFEXITED(p->status)) snprintf(out, cap, "%s:%d", p->name, WEXITSTATUS(p->status));
    else if (p->exited && WIFSIGNALED(p->status)) snprintf(out, cap, "%s:sig%d", p->name, WTERMSIG(p->status));
    else snprintf(out, cap, "%s:x", p->name);
}

static int exited_zero(const proc_t *p) { return p->exited && WIFEXITED(p->status) && WEXITSTATUS(p->status) == 0; }

/* ---- loopback experiment ---- */

typedef struct {
    const char *name;
    const char *server_bin;
    const char *mode;
    int hb_thread;
    int per_op_conn;
} topo_t;

static const topo_t TOPOS[] = {
    {"c3e", "dskv", "epoll", 0, 0},   {"c3t", "dskv", "timeout", 0, 0}, {"c3t2", "dskv", "timeout", 1, 0},
    {"c3b", "dskv", "block", 0, 1},   {"g3", "dsgo", NULL, 0, 0},       {"r3", "dsrs", NULL, 0, 0},
};

static int run_n0(const char *topo_name, int port_base) {
    const topo_t *topo = NULL;
    for (size_t i = 0; i < sizeof TOPOS / sizeof TOPOS[0]; i++)
        if (strcmp(TOPOS[i].name, topo_name) == 0) topo = &TOPOS[i];
    if (topo == NULL) ds_die("unknown topology");
    setup_trial_files(0);
    maybe_leak_child();

    proc_t ps[5];
    const char *server_names[3] = {"n1", "n2", "n3"};
    const char *client_names[2] = {"c1", "c2"};
    for (int s = 0; s < 3; s++) {
        argv_t a;
        av_init(&a, topo->server_bin);
        if (topo->mode != NULL) av_add(&a, "--mode=%s", topo->mode);
        av_add(&a, "--node=%s", server_names[s]);
        av_add(&a, "--port=%d", port_base + s);
        av_add(&a, "--peer-port=%d", port_base + (s + 1) % 3);
        av_add(&a, "--tmp=%s", g_tmp);
        if (topo->hb_thread) av_add(&a, "--hb-thread");
        for (int e = 0; e < g_extra_count; e++)
            if (strcmp(g_extra[e].node, server_names[s]) == 0) av_add(&a, "%s", g_extra[e].arg);
        spawn(&ps[s], server_names[s], &a, NULL);
    }
    for (int c = 0; c < 2; c++) {
        argv_t a;
        av_init(&a, "dscli");
        av_add(&a, "--node=%s", client_names[c]);
        av_add(&a, "--servers=%d,%d,%d", port_base, port_base + 1, port_base + 2);
        av_add(&a, "--tmp=%s", g_tmp);
        if (topo->per_op_conn) av_add(&a, "--per-op-conn");
        for (int e = 0; e < g_extra_count; e++)
            if (strcmp(g_extra[e].node, client_names[c]) == 0) av_add(&a, "%s", g_extra[e].arg);
        spawn(&ps[3 + c], client_names[c], &a, NULL);
    }

    unsigned stragglers = 0;
    wait_all(&ps[3], 2, CLIENT_POLLS, CLIENT_POLL_MS);
    stragglers += kill_stragglers(&ps[3], 2);
    for (int s = 0; s < 3; s++)
        if (ps[s].alive) send_quit(port_base + s);
    wait_all(&ps[0], 3, SERVER_POLLS, SERVER_POLL_MS);
    stragglers += kill_stragglers(&ps[0], 3);

    uint64_t digest;
    unsigned count;
    int seq_ok = check_history(client_names, 2, 2 * N0_OPS * 2, &digest, &count);
    int live_after = has_live_child();
    int all_zero = 1;
    char exits[128] = "";
    for (int i = 0; i < 5; i++) {
        char one[32];
        format_status(one, sizeof one, &ps[i]);
        size_t used = strlen(exits);
        snprintf(exits + used, sizeof exits - used, "%s%s", i ? "," : "", one);
        if (!exited_zero(&ps[i])) all_zero = 0;
    }
    int ok = all_zero && stragglers == 0 && !live_after && seq_ok;
    ds_emit("DSN0 v1 role=launch servers=3 clients=2 exits=%s stragglers=%u live_after=%d seq=%u seq_ok=%d "
            "hist=%016llx ok=%d\n",
            exits, stragglers, live_after, count, seq_ok, (unsigned long long)digest, ok);
    return ok ? 0 : 3;
}

/* ---- crash experiment ---- */

typedef struct {
    unsigned d[MAX_KILLS];
    unsigned r[MAX_KILLS];
} plan_t;

static uint64_t draw(uint64_t seed, uint64_t index) { return ds_mix(seed ^ ds_mix(CRASH_SALT ^ index)); }

static plan_t crash_plan(uint64_t seed, int kills) {
    plan_t p;
    memset(&p, 0, sizeof p);
    for (int j = 1; j <= kills; j++) {
        p.r[j - 1] = 20 + (unsigned)(draw(seed, 2 * (uint64_t)j - 1) % 61);
        if (j == 1) p.d[0] = 40 + (unsigned)(draw(seed, 0) % 241);
        else p.d[j - 1] = p.d[j - 2] + p.r[j - 2] + 30 + (unsigned)(draw(seed, 2) % 121);
    }
    return p;
}

static void print_plan(uint64_t seed, int kills) {
    plan_t p = crash_plan(seed, kills);
    for (int j = 0; j < kills; j++) ds_emit("kill=%d planned_ms=%u restart_ms=%u\n", j + 1, p.d[j], p.r[j]);
}

typedef struct {
    int use_group;
    int kills;
    int port;
    int helper;
    int two_threads;
    int pause;
    int raw;
    unsigned pace_ms;
    const char *node_bin;
    uint64_t seed;
} crash_cfg_t;

static void spawn_node(proc_t *p, const crash_cfg_t *cfg, int incarnation) {
    char tag[24];
    snprintf(tag, sizeof tag, "n1.i%d", incarnation);
    argv_t a;
    av_init(&a, cfg->node_bin);
    av_add(&a, "--node=n1");
    av_add(&a, "--port=%d", cfg->port);
    av_add(&a, "--tmp=%s", g_tmp);
    if (cfg->two_threads) av_add(&a, "--threads=2");
    if (cfg->pause) av_add(&a, "--pause");
    spawn(p, "n1", &a, tag);
    snprintf(p->tag, sizeof p->tag, "%s", tag);
}

/* The helper of a process-group trial: a copy of this launcher that does not exec, joins the node's process group and
 * loops on a 5 ms sleep for at most about 120 s. It watches neither its parent nor the node: only the group kill should
 * end it, so a kill that misses it stays visible. */
static void spawn_helper(proc_t *h, pid_t node_pid) {
    pid_t pid = fork();
    if (pid < 0) ds_die("fork failed");
    if (pid == 0) {
        if (setpgid(0, node_pid) != 0) _exit(2);
        for (int i = 0; i < HELPER_LOOPS; i++) ds_sleep_ms(5);
        _exit(0);
    }
    memset(h, 0, sizeof *h);
    h->pid = pid;
    h->alive = 1;
    snprintf(h->name, sizeof h->name, "h");
    snprintf(h->tag, sizeof h->tag, "n1.i0.h");
}

static unsigned g_unexpected;

/* One "exited" line per child, ever. A line is expected only as a planned kill that ended in SIGKILL, or as a normal exit
 * with status 0 of the client or of the final incarnation. */
static void report_exited(proc_t *p) {
    if (p->reported) return;
    p->reported = 1;
    const char *cause;
    int status = -1;
    if (!p->exited) {
        cause = "unreaped";
    } else {
        status = p->status;
        if (p->stop_killed) cause = "stop_kill";
        else if (p->planned_sent && WIFSIGNALED(status) && WTERMSIG(status) == 9) cause = "planned_kill";
        else cause = WIFEXITED(status) ? "exit" : "signal";
    }
    int expected = (strcmp(cause, "planned_kill") == 0 && status == 9) ||
                   (strcmp(cause, "exit") == 0 && status == 0 && p->expect_clean_exit);
    if (!expected) g_unexpected++;
    ds_emit("HAIRSPRING_EVENT {\"v\":1,\"kind\":\"exited\",\"proc\":\"_launcher\",\"target\":\"%s\",\"status\":%d,"
            "\"cause\":\"%s\"}\n",
            p->tag, status, cause);
}

/* SIGKILL by pid, reap (at most 1,000 polls of 1 ms), remember that the launcher stopped it. */
static void stop_kill(proc_t *p) {
    if (!p->alive) return;
    p->stop_killed = 1;
    kill(p->pid, SIGKILL);
    for (int k = 0; k < STRAGGLER_REAP_POLLS; k++) {
        if (reap_once(p)) return;
        ds_sleep_ms(1);
    }
}

typedef struct {
    int sent;      /* a kill was sent */
    int pre_dead;  /* the victim was already gone at the alive check */
    int confirmed; /* sent, and the victim node was reaped with SIGKILL */
    int unreaped;  /* sent, but the victim was not reaped in time */
    int grp_gone;  /* -1: not applicable */
} kill_result_t;

/* One planned kill, in this order: yield (not in the raw trial), alive check, kill with no system call between the check
 * and the kill (in the raw trial: alive check, one write, kill), reap, group check, lines. */
static kill_result_t do_kill(proc_t *victim, proc_t *helper, const crash_cfg_t *cfg, int j, unsigned planned_ms,
                             int incarnation, int region_seen) {
    kill_result_t res = {0, 0, 0, 0, cfg->use_group ? 0 : -1};
    int st = 0;
    if (!cfg->raw) sched_yield();
    pid_t ar = ds_wait_child(victim, &st);
    if (ar == 0) {
        if (cfg->raw) {
            ssize_t w = write(1, PRE_KILL_LINE, sizeof PRE_KILL_LINE - 1);
            (void)w;
        }
#ifdef DS_MUTANT_PID_KILL
        /* Regression mutant (built only by the native tests): a group-kill trial that kills the pid alone. */
        kill(victim->pid, SIGKILL);
#else
        kill(cfg->use_group ? -victim->pid : victim->pid, SIGKILL);
#endif
        res.sent = 1;
    } else if (ar == victim->pid) {
        victim->alive = 0;
        victim->exited = 1;
        victim->status = st;
        res.pre_dead = 1;
    } else {
        victim->alive = 0; /* an alive-check error counts as "not alive": never signalled or waited for again */
        res.pre_dead = 1;
    }
    victim->planned_sent = res.sent;
    if (helper != NULL) helper->planned_sent = res.sent;

    unsigned polls = 0;
    if (res.sent) {
        while (polls < KILL_REAP_POLLS) {
            polls++;
            int v = reap_once(victim);
            int h = helper == NULL || reap_once(helper);
            if (v && h) break;
            ds_sleep_ms(1);
        }
        if (cfg->use_group && victim->exited && (helper == NULL || helper->exited)) {
            for (int k = 0; k < GROUP_GONE_POLLS; k++) {
                if (kill(-victim->pid, 0) < 0 && errno == ESRCH) {
                    res.grp_gone = 1;
                    break;
                }
                ds_sleep_ms(1);
            }
        }
    }
    int wifsig = victim->exited && WIFSIGNALED(victim->status);
    int termsig = wifsig ? WTERMSIG(victim->status) : 0;
    res.confirmed = res.sent && wifsig && termsig == 9;
    res.unreaped = res.sent && !victim->exited;

    if (res.confirmed) {
        ds_emit("HAIRSPRING_EVENT {\"v\":1,\"kind\":\"killed\",\"proc\":\"_launcher\",\"target\":\"n1.i%d\",\"signal\":9,"
                "\"seq\":%d,\"point\":\"t%u\"}\n",
                incarnation, j, planned_ms);
    }
    if (victim->exited || res.pre_dead) report_exited(victim);
    if (helper != NULL && helper->exited) report_exited(helper);
    char grp[4] = "na", seen[4] = "na";
    if (res.grp_gone >= 0) snprintf(grp, sizeof grp, "%d", res.grp_gone);
    if (region_seen >= 0) snprintf(seen, sizeof seen, "%d", region_seen);
    ds_emit("DSC0 v1 role=sup kill=%d victim=n1.i%d pre_dead=%d wifsig=%d termsig=%d reap_polls=%u grp_gone=%s "
            "region_seen=%s planned_ms=%u\n",
            j, incarnation, res.pre_dead, wifsig, termsig, polls, grp, seen, planned_ms);
    if (helper != NULL && res.sent && !helper->exited) {
        stop_kill(helper);
        report_exited(helper);
    }
    return res;
}

static int region_file_exists(int incarnation) {
    char rel[64], path[512];
    struct stat st;
    snprintf(rel, sizeof rel, "hairspring-region/n1.i%d", incarnation);
    path_in_tmp(path, sizeof path, rel);
    return stat(path, &st) == 0;
}

static int run_c0(const crash_cfg_t *cfg) {
    setup_trial_files(1);
    maybe_leak_child();
    plan_t plan = crash_plan(cfg->seed, cfg->kills);
    proc_t nodes[MAX_KILLS + 1], helper, client;
    memset(nodes, 0, sizeof nodes);
    memset(&helper, 0, sizeof helper);
    memset(&client, 0, sizeof client);
    unsigned stragglers = 0, restarts = 0;
    int incarnation = 0, client_started = 0;

    spawn_node(&nodes[0], cfg, 0);
#ifdef DS_STUB_WAITPID
    nodes[0].stub = 1;
#endif
    if (cfg->helper) spawn_helper(&helper, nodes[0].pid);
    if (!cfg->pause) {
        argv_t a;
        av_init(&a, "dscli");
        av_add(&a, "--node=c1");
        av_add(&a, "--servers=%d", cfg->port);
        av_add(&a, "--tmp=%s", g_tmp);
        av_add(&a, "--reconnect");
        if (cfg->pace_ms > 0) av_add(&a, "--pace-ms=%u", cfg->pace_ms);
        spawn(&client, "c1", &a, NULL);
        snprintf(client.tag, sizeof client.tag, "c1");
        client_started = 1;
    }

    unsigned wait_ms = cfg->raw ? KW_WAIT_MS : plan.d[0];
    int kills_done = 0, all_clean = 1, grp_ok = 1, region_seen = -1, plan_stopped = 0;
    for (int j = 1; j <= cfg->kills; j++) {
        ds_sleep_ms(wait_ms);
        if (cfg->raw) region_seen = region_file_exists(incarnation);
        unsigned planned = cfg->raw ? KW_PLANNED_MS : plan.d[j - 1];
        proc_t *h = incarnation == 0 && cfg->helper ? &helper : NULL;
        kill_result_t res = do_kill(&nodes[incarnation], h, cfg, j, planned, incarnation, region_seen);
        kills_done++;
        if (res.pre_dead || !res.confirmed) all_clean = 0;
        if (cfg->use_group && res.grp_gone != 1) grp_ok = 0;
        if (res.unreaped) {
            plan_stopped = 1; /* never start the next incarnation over a victim that may still be running */
            break;
        }
        if (cfg->raw) break;
        ds_sleep_ms(plan.r[j - 1]);
        incarnation++;
        restarts++;
        spawn_node(&nodes[incarnation], cfg, incarnation);
        if (j < cfg->kills) wait_ms = plan.d[j] - plan.d[j - 1] - plan.r[j - 1];
    }

    /* Teardown. */
    if (client_started) {
        wait_all(&client, 1, CLIENT_POLLS, CLIENT_POLL_MS);
        if (client.alive) stragglers++;
        stop_kill(&client);
        client.expect_clean_exit = 1;
        report_exited(&client);
    }
    proc_t *live = &nodes[incarnation];
    if (!cfg->raw && !plan_stopped) {
        live->expect_clean_exit = 1;
        if (live->alive) send_quit(cfg->port);
        wait_all(live, 1, SERVER_POLLS, SERVER_POLL_MS);
        if (live->alive) stragglers++;
        stop_kill(live);
    } else {
        /* The raw trial's victim is already reaped; after an unreaped victim, whatever is left is stopped again. */
        if (live->alive) stragglers++;
        stop_kill(live);
    }
    report_exited(live);
    if (cfg->helper) {
        stop_kill(&helper);
        report_exited(&helper);
    }

    const char *clients[1] = {"c1"};
    uint64_t digest;
    unsigned count;
    int seq_ok = check_history(clients, 1, client_started ? 2 * C0_OPS : 0, &digest, &count);
    int live_after = has_live_child();
    int ok = kills_done == cfg->kills && all_clean && (int)restarts == (cfg->raw ? 0 : cfg->kills) && g_unexpected == 0 &&
             stragglers == 0 && !live_after && seq_ok && grp_ok && (!cfg->raw || region_seen == 1);
    ds_emit("DSC0 v1 role=launch kills=%d restarts=%u stragglers=%u unexpected=%u live_after=%d seq_ok=%d hist=%016llx "
            "ok=%d\n",
            kills_done, restarts, stragglers, g_unexpected, live_after, seq_ok, (unsigned long long)digest, ok);
    return ok ? 0 : 3;
}

/* ---- arguments ---- */

static void parse_extras(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        if (strncmp(argv[i], "--server-arg=", 13) != 0) continue;
        const char *spec = argv[i] + 13;
        const char *colon = strchr(spec, ':');
        if (colon == NULL || colon - spec >= (long)sizeof g_extra[0].node || g_extra_count >= MAX_EXTRA)
            ds_die("bad --server-arg");
        snprintf(g_extra[g_extra_count].node, sizeof g_extra[0].node, "%.*s", (int)(colon - spec), spec);
        snprintf(g_extra[g_extra_count].arg, sizeof g_extra[0].arg, "%s", colon + 1);
        g_extra_count++;
    }
}

static uint64_t seed_from(int argc, char **argv) {
    const char *s = ds_opt(argc, argv, "--seed=");
    if (s == NULL) ds_die("a seed is needed: --seed=S");
    return ds_parse_u64(s, "bad seed");
}

int main(int argc, char **argv) {
    if (ds_flag(argc, argv, "--print-plan")) {
        const char *k = ds_opt(argc, argv, "--kills=");
        if (k == NULL) ds_die("usage: dsl --print-plan --seed=S --kills=K");
        uint64_t kills = ds_parse_u64(k, "bad --kills");
        if (kills < 1 || kills > MAX_KILLS) ds_die("--kills must be 1 or 2");
        print_plan(seed_from(argc, argv), (int)kills);
        return 0;
    }
    g_bin_dir = ds_opt(argc, argv, "--bin-dir=");
    g_tmp = ds_opt(argc, argv, "--tmp=");
    if (g_bin_dir == NULL || g_tmp == NULL) ds_die("usage: dsl (--topo=T | --crash=pid,K) --bin-dir=DIR --tmp=DIR ...");
    parse_extras(argc, argv);
    g_leak_child = ds_flag(argc, argv, "--test-leak-child");

    const char *topo = ds_opt(argc, argv, "--topo=");
    if (topo != NULL) {
        const char *base = ds_opt(argc, argv, "--port-base=");
        return run_n0(topo, base != NULL ? ds_parse_port(base) : 7101);
    }

    const char *crash = ds_opt(argc, argv, "--crash=");
    const char *port = ds_opt(argc, argv, "--port=");
    if (crash == NULL || port == NULL) ds_die("give --topo=T or --crash=pid,K with --port=P");
    crash_cfg_t cfg;
    memset(&cfg, 0, sizeof cfg);
    const char *comma = strchr(crash, ',');
    if (comma == NULL) ds_die("--crash must look like pid,1");
    if (strncmp(crash, "pgid", (size_t)(comma - crash)) == 0 && comma - crash == 4) cfg.use_group = 1;
    else if (!(comma - crash == 3 && strncmp(crash, "pid", 3) == 0)) ds_die("--crash kind must be pid or pgid");
    uint64_t kills = ds_parse_u64(comma + 1, "bad kill count");
    if (kills < 1 || kills > MAX_KILLS) ds_die("kill count must be 1 or 2");
    cfg.kills = (int)kills;
    cfg.port = ds_parse_port(port);
    cfg.helper = ds_flag(argc, argv, "--helper");
    cfg.two_threads = ds_opt(argc, argv, "--threads=") != NULL;
    cfg.pause = ds_flag(argc, argv, "--pause");
    cfg.raw = ds_flag(argc, argv, "--raw-write-kill");
    const char *pace = ds_opt(argc, argv, "--pace-ms=");
    cfg.pace_ms = pace != NULL ? (unsigned)ds_parse_u64(pace, "bad --pace-ms") : DEFAULT_PACE_MS;
    const char *node_bin = ds_opt(argc, argv, "--node-bin=");
    cfg.node_bin = node_bin != NULL ? node_bin : "dsnode";
    cfg.seed = seed_from(argc, argv);
    if (cfg.raw && (!cfg.pause || cfg.kills != 1)) ds_die("--raw-write-kill needs --pause and one kill");
    return run_c0(&cfg);
}
