/* test_proc.c -- process-level tests: run the sample program and inspect the events it writes.
** Usage: test_proc <dir-with-hs_sample_binaries>
*/
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

#include "hs_test.h"

#define MAX_LINES 4096
#define MAX_OUT (4 * 1024 * 1024)
#define LINE_CAP 4096
#define PREFIX "HAIRSPRING_EVENT "

#if defined(__SANITIZE_ADDRESS__)
#define HS_T_ASAN 1
#elif defined(__has_feature)
#if __has_feature(address_sanitizer)
#define HS_T_ASAN 1
#endif
#endif

static const char *g_dir; /* directory holding the hs_sample_<scenario> binaries */

struct capture {
  char *buf;
  size_t len;
  int status;
  char *lines[MAX_LINES];
  size_t n_lines;
};

static void split_lines(struct capture *c) {
  c->n_lines = 0;
  char *p = c->buf;
  while (p < c->buf + c->len && c->n_lines < MAX_LINES) {
    char *nl = memchr(p, '\n', (size_t)(c->buf + c->len - p));
    if (!nl) break;
    *nl = 0;
    c->lines[c->n_lines++] = p;
    p = nl + 1;
  }
}

/* Child setup: stderr to `err_fd`, stdout to /dev/null, extra environment as NAME=VALUE pairs. */
static pid_t spawn(const char *scenario, const char *arg, const char *const *env, int err_fd) {
  pid_t pid = fork();
  if (pid != 0) return pid;
  int devnull = open("/dev/null", O_WRONLY);
  dup2(devnull, 1);
  dup2(err_fd, 2);
  for (; env && *env; env++) {
    char name[64], *eq = strchr(*env, '=');
    size_t n = (size_t)(eq - *env);
    memcpy(name, *env, n);
    name[n] = 0;
    setenv(name, eq + 1, 1);
  }
  char path[512];
  snprintf(path, sizeof path, "%s/hs_sample_%s", g_dir, scenario);
  if (getenv("HS_TEST_IGNORE_TERM")) { /* test hook: SIG_IGN survives the exec, as with `trap '' TERM` */
    signal(SIGTERM, SIG_IGN);
    unsetenv("HS_TEST_IGNORE_TERM");
  }
  const char *nofile = getenv("HS_TEST_NOFILE"); /* test hook: shrink the descriptor limit before the exec */
  if (nofile) {
    struct rlimit lim = {(rlim_t)atoi(nofile), (rlim_t)atoi(nofile)};
    setrlimit(RLIMIT_NOFILE, &lim);
    unsetenv("HS_TEST_NOFILE");
  }
  execl(path, path, arg, (char *)NULL);
  _exit(127);
}

static size_t read_all(int fd, char *buf, size_t cap) {
  size_t len = 0;
  for (;;) {
    ssize_t r = read(fd, buf + len, cap - len);
    if (r < 0 && errno == EINTR) continue;
    if (r <= 0) break;
    len += (size_t)r;
  }
  return len;
}

static struct capture *run_capture(const char *scenario, const char *arg, const char *const *env) {
  struct capture *c = calloc(1, sizeof *c);
  c->buf = malloc(MAX_OUT);
  int fds[2];
  if (pipe(fds) != 0) abort();
  pid_t pid = spawn(scenario, arg, env, fds[1]);
  close(fds[1]);
  c->len = read_all(fds[0], c->buf, MAX_OUT - 1);
  close(fds[0]);
  waitpid(pid, &c->status, 0);
  c->buf[c->len] = 0;
  split_lines(c);
  return c;
}

static void free_capture(struct capture *c) {
  free(c->buf);
  free(c);
}

/* ---- line inspection ------------------------------------------------------------------------------------------ */

static int has(const char *line, const char *needle) { return strstr(line, needle) != NULL; }

/* True when any line contains `needle` (the buffer itself was cut into lines, so strstr on it sees only one). */
static int any_line_has(const struct capture *c, const char *needle) {
  for (size_t i = 0; i < c->n_lines; i++)
    if (has(c->lines[i], needle)) return 1;
  return 0;
}

static int is_kind(const char *line, const char *kind) {
  char needle[40];
  snprintf(needle, sizeof needle, "\"kind\":\"%s\"", kind);
  return has(line, needle);
}

static int on_proc(const char *line, const char *proc) {
  char needle[96];
  snprintf(needle, sizeof needle, "\"proc\":\"%s\"", proc);
  return has(line, needle);
}

static size_t count(const struct capture *c, const char *kind, const char *proc) {
  size_t n = 0;
  for (size_t i = 0; i < c->n_lines; i++)
    if (is_kind(c->lines[i], kind) && (!proc || on_proc(c->lines[i], proc))) n++;
  return n;
}

static const char *find_line(const struct capture *c, const char *kind, const char *proc, const char *id) {
  char idn[200];
  snprintf(idn, sizeof idn, "\"id\":\"%s\"", id ? id : "");
  for (size_t i = 0; i < c->n_lines; i++) {
    const char *l = c->lines[i];
    if (is_kind(l, kind) && on_proc(l, proc) && (!id || has(l, idn))) return l;
  }
  return NULL;
}

static int64_t field(const char *line, const char *name) {
  if (!line) return -1;
  char needle[48];
  snprintf(needle, sizeof needle, "\"%s\":", name);
  const char *p = strstr(line, needle);
  if (!p) return -1;
  p += strlen(needle);
  while (*p == ' ') p++;
  return (int64_t)strtoll(p, NULL, 10);
}

/* The whole line is one balanced JSON object: braces balance only at the very last character. */
static int is_whole_object(const char *line) {
  if (strncmp(line, PREFIX, strlen(PREFIX)) != 0) return 0;
  const char *p = line + strlen(PREFIX);
  int depth = 0, in_str = 0;
  for (; *p; p++) {
    if (in_str) {
      if (*p == '\\') p++;
      else if (*p == '"') in_str = 0;
      continue;
    }
    if (*p == '"') in_str = 1;
    else if (*p == '{') depth++;
    else if (*p == '}') {
      depth--;
      if (depth == 0) return p[1] == 0;
    }
  }
  return 0;
}

static void check_all_lines_wellformed(const struct capture *c) {
  for (size_t i = 0; i < c->n_lines; i++) {
    CHECK(strlen(c->lines[i]) + 1 <= LINE_CAP);
    CHECK(is_whole_object(c->lines[i]));
    if (!is_whole_object(c->lines[i])) printf("  bad line %zu: %.120s\n", i, c->lines[i]);
  }
}

/* ---- tests ---------------------------------------------------------------------------------------------------- */

static void test_basic_report_shape(void) {
  struct capture *c = run_capture("basic", NULL, NULL);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0);
  check_all_lines_wellformed(c);
  CHECK(c->n_lines > 0 && is_kind(c->lines[0], "hello"));
  CHECK(c->n_lines > 0 && is_kind(c->lines[c->n_lines - 1], "end"));
  CHECK_U64(count(c, "hello", "p1"), 1);
  CHECK_U64(count(c, "catalog", "p1"), 7);
  CHECK_U64(count(c, "assert", "p1"), 7);
  CHECK_U64(count(c, "work", "p1"), 1);
  CHECK_U64(count(c, "end", "p1"), 1);
  CHECK(has(c->lines[0], "\"signal_handlers\":false"));
  CHECK(has(c->lines[0], "\"sdk\":\"c\""));

  const char *ok = find_line(c, "assert", "p1", "basic.always.ok");
  CHECK_U64(field(ok, "hits"), 5);
  CHECK_U64(field(ok, "trues"), 5);
  CHECK_U64(field(ok, "falses"), 0);
  CHECK(ok && has(ok, "\"first_false\":null"));

  const char *bad = find_line(c, "assert", "p1", "basic.always.bad");
  CHECK_U64(field(bad, "falses"), 1);
  CHECK(bad && has(bad, "\"vals\":[7,8,0,0]"));

  const char *never = find_line(c, "assert", "p1", "basic.reach.never");
  CHECK_U64(field(never, "hits"), 0);
  const char *reach = find_line(c, "assert", "p1", "basic.reach.hit");
  CHECK_U64(field(reach, "hits"), 1);
  CHECK(reach && !has(reach, "first_false"));

  CHECK_U64(field(find_line(c, "work", "p1", NULL), "ops"), 300);
  const char *end = find_line(c, "end", "p1", NULL);
  CHECK_U64(field(end, "sites"), 7);
  CHECK_U64(field(end, "threads"), 1);
  CHECK(end && has(end, "\"reason\":\"exit\""));
  free_capture(c);
}

static void test_same_id_at_two_call_sites_is_one_assert_line(void) {
  struct capture *c = run_capture("merge", NULL, NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "assert", "p1"), 2);
  CHECK_U64(count(c, "catalog", "p1"), 2);
  const char *shared = find_line(c, "assert", "p1", "merge.shared");
  CHECK_U64(field(shared, "hits"), 4);
  CHECK_U64(field(shared, "trues"), 3);
  CHECK_U64(field(shared, "falses"), 1);
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "sites"), 2);
  free_capture(c);
}

static void test_fork_child_gets_derived_tag_and_fresh_counters(void) {
  struct capture *c = run_capture("fork", NULL, NULL);
  check_all_lines_wellformed(c);
  const char *tags[] = {"p1", "p1.f1", "p1.f2"};
  for (int i = 0; i < 3; i++) {
    CHECK_U64(count(c, "hello", tags[i]), 1);
    CHECK_U64(count(c, "end", tags[i]), 1);
    CHECK_U64(count(c, "assert", tags[i]), 2);
  }
  CHECK_U64(field(find_line(c, "assert", "p1", "fork.parent"), "hits"), 2);
  CHECK_U64(field(find_line(c, "assert", "p1", "fork.child"), "hits"), 0);
  for (int i = 1; i < 3; i++) {
    CHECK_U64(field(find_line(c, "assert", tags[i], "fork.parent"), "hits"), 0);
    CHECK_U64(field(find_line(c, "assert", tags[i], "fork.child"), "hits"), 3);
  }
  free_capture(c);
}

static void test_many_threads_each_get_a_thread_line(void) {
  struct capture *c = run_capture("threads", "100", NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "thread", "p1"), 101);
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "threads"), 101);
  CHECK(find_line(c, "thread", "p1", NULL) != NULL);
  CHECK(any_line_has(c, "\"thread\":\"w-99\""));
  CHECK(any_line_has(c, "\"key_stable\":true"));
  CHECK(any_line_has(c, "\"key_stable\":false"));
  CHECK_U64(field(find_line(c, "assert", "p1", "threads.each"), "hits"), 100);
  free_capture(c);
}

static void test_threads_beyond_the_table_are_capped_and_counted(void) {
  struct capture *c = run_capture("threads", "300", NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "thread", "p1"), 256);
  const char *end = find_line(c, "end", "p1", NULL);
  CHECK_U64(field(end, "threads"), 256);
  CHECK(field(end, "dropped") > 0);
  CHECK(any_line_has(c, "\"thread\":\"t#overflow\""));
  free_capture(c);
}

static void test_signal_report_is_opt_in(void) {
  struct capture *c;
#ifdef HS_T_ASAN
  /* ASan owns SIGSEGV; the SDK leaves an existing handler alone, so the opt-in report cannot be observed. */
  printf("skip: signal report for SIGSEGV under AddressSanitizer\n");
#else
  const char *on[] = {"HAIRSPRING_SIGNAL_REPORT=1", NULL};
  c = run_capture("crash", NULL, on);
  CHECK(WIFSIGNALED(c->status) && WTERMSIG(c->status) == SIGSEGV);
  check_all_lines_wellformed(c);
  CHECK(has(c->lines[0], "\"signal_handlers\":true"));
  const char *end = find_line(c, "end", "p1", NULL);
  CHECK(end && has(end, "\"reason\":\"signal\""));
  CHECK_U64(field(end, "status"), SIGSEGV);
  CHECK_U64(field(find_line(c, "assert", "p1", "crash.before"), "hits"), 1);
  CHECK_U64(count(c, "end", "p1"), 1);
  free_capture(c);
#endif

  c = run_capture("crash", NULL, NULL);
  CHECK(WIFSIGNALED(c->status) && WTERMSIG(c->status) == SIGSEGV);
  CHECK(has(c->lines[0], "\"signal_handlers\":false"));
  CHECK_U64(count(c, "catalog", "p1"), 2); /* the catalog is out before the crash... */
  CHECK_U64(count(c, "end", "p1"), 0);     /* ...but nothing else: the engine reports report_missing */
  CHECK_U64(count(c, "assert", "p1"), 0);
  free_capture(c);
}

static void test_exit_status_is_recorded_by_hs_exit(void) {
  struct capture *c = run_capture("exit_status", NULL, NULL);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 3);
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "status"), 3);
  free_capture(c);
}

static void test_events_file_receives_the_report_instead_of_stderr(void) {
  char path[] = "/tmp/hs_events_XXXXXX";
  int fd = mkstemp(path);
  CHECK(fd >= 0);
  close(fd);
  char setting[128];
  snprintf(setting, sizeof setting, "HAIRSPRING_EVENTS_FILE=%s", path);
  const char *env[] = {setting, NULL};
  struct capture *c = run_capture("basic", NULL, env);
  CHECK_U64(c->n_lines, 0);

  struct capture file = {0};
  file.buf = malloc(MAX_OUT);
  int rd = open(path, O_RDONLY);
  file.len = read_all(rd, file.buf, MAX_OUT - 1);
  close(rd);
  file.buf[file.len] = 0;
  split_lines(&file);
  check_all_lines_wellformed(&file);
  CHECK_U64(count(&file, "assert", "p1"), 7);
  CHECK_U64(count(&file, "end", "p1"), 1);
  free(file.buf);
  unlink(path);
  free_capture(c);
}

static void test_events_off_writes_nothing(void) {
  const char *env[] = {"HAIRSPRING_EVENTS=off", NULL};
  struct capture *c = run_capture("basic", NULL, env);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0);
  CHECK_U64(c->len, 0);
  free_capture(c);
}

static void test_eval_forms_write_one_line_each(void) {
  struct capture *c = run_capture("eval", NULL, NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "eval", NULL), 3);
  CHECK(any_line_has(c, "\"kind\":\"eval\",\"id\":\"eval.always\",\"type\":\"always\",\"ok\":true"));
  CHECK(any_line_has(c, "\"id\":\"eval.sometimes\",\"type\":\"sometimes\",\"ok\":false"));
  free_capture(c);
}

/* POSIX promises atomic pipe writes only up to PIPE_BUF: 4096 on Linux, 512 on macOS. The report's longest lines run
** to about 600 bytes (two padded witnesses), so on macOS two processes share an O_APPEND events file instead; the
** lines are not shortened. */
#define PIPE_SITES 64

static void read_file_into(struct capture *c, const char *path) {
  int rd = open(path, O_RDONLY);
  c->len = read_all(rd, c->buf, MAX_OUT - 1);
  close(rd);
  c->buf[c->len] = 0;
  split_lines(c);
}

static void test_two_processes_sharing_one_output_never_interleave_inside_a_line(void) {
  for (int round = 0; round < 20; round++) {
    struct capture *c = calloc(1, sizeof *c);
    c->buf = malloc(MAX_OUT);
#if defined(__linux__)
    int fds[2];
    CHECK(pipe(fds) == 0);
    const char *env_a[] = {"HAIRSPRING_PROC=pA", NULL};
    const char *env_b[] = {"HAIRSPRING_PROC=pB", NULL};
    pid_t a = spawn("big", NULL, env_a, fds[1]);
    pid_t b = spawn("big", NULL, env_b, fds[1]);
    close(fds[1]);
    c->len = read_all(fds[0], c->buf, MAX_OUT - 1);
    close(fds[0]);
    c->buf[c->len] = 0;
    split_lines(c);
    waitpid(a, NULL, 0);
    waitpid(b, NULL, 0);
#else
    char path[] = "/tmp/hs_shared_XXXXXX";
    int tmp = mkstemp(path);
    CHECK(tmp >= 0);
    close(tmp);
    char file_env[128];
    snprintf(file_env, sizeof file_env, "HAIRSPRING_EVENTS_FILE=%s", path);
    const char *env_a[] = {"HAIRSPRING_PROC=pA", file_env, NULL};
    const char *env_b[] = {"HAIRSPRING_PROC=pB", file_env, NULL};
    int devnull = open("/dev/null", O_WRONLY);
    pid_t a = spawn("big", NULL, env_a, devnull);
    pid_t b = spawn("big", NULL, env_b, devnull);
    close(devnull);
    waitpid(a, NULL, 0);
    waitpid(b, NULL, 0);
    read_file_into(c, path);
    unlink(path);
#endif
    check_all_lines_wellformed(c);
    const char *procs[] = {"pA", "pB"};
    for (int i = 0; i < 2; i++) {
      CHECK_U64(count(c, "hello", procs[i]), 1);
      CHECK_U64(count(c, "catalog", procs[i]), PIPE_SITES);
      CHECK_U64(count(c, "assert", procs[i]), PIPE_SITES);
      CHECK_U64(count(c, "end", procs[i]), 1);
    }
    CHECK_U64(c->n_lines, 2 * (1 + PIPE_SITES + PIPE_SITES + 1 + 1 + 1));
    free_capture(c);
  }
}

/* ---- review findings ------------------------------------------------------------------------------------------ */

static void test_events_survive_stderr_being_closed_and_reused(void) {
  char path[] = "/tmp/hs_data_XXXXXX";
  int tmp = mkstemp(path);
  CHECK(tmp >= 0);
  close(tmp);
  struct capture *c = run_capture("closefd", path, NULL);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0);
  CHECK_U64(count(c, "end", "p1"), 1); /* the report still reached the original stderr... */
  struct capture data = {0};
  data.buf = malloc(MAX_OUT);
  read_file_into(&data, path);
  CHECK_U64(data.len, 17); /* ...and the program's own file received only its own bytes */
  CHECK(!any_line_has(&data, "HAIRSPRING_EVENT"));
  free(data.buf);
  unlink(path);
  free_capture(c);
}

static void test_events_survive_stderr_being_closed_without_reuse(void) {
  struct capture *c = run_capture("closefd", NULL, NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "end", "p1"), 1);
  free_capture(c);
}

static void test_unwritable_events_file_falls_back_to_stderr_and_is_counted(void) {
  const char *env[] = {"HAIRSPRING_EVENTS_FILE=/nonexistent-dir/events", NULL};
  struct capture *c = run_capture("basic", NULL, env);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "end", "p1"), 1);
  CHECK(field(find_line(c, "end", "p1", NULL), "dropped") >= 1);
  free_capture(c);
}

static void test_events_file_symlink_is_not_followed(void) {
  char target[] = "/tmp/hs_target_XXXXXX";
  int tmp = mkstemp(target);
  CHECK(tmp >= 0);
  close(tmp);
  char link_path[64];
  snprintf(link_path, sizeof link_path, "%s.link", target);
  CHECK(symlink(target, link_path) == 0);
  char setting[128];
  snprintf(setting, sizeof setting, "HAIRSPRING_EVENTS_FILE=%s", link_path);
  const char *env[] = {setting, NULL};
  struct capture *c = run_capture("basic", NULL, env);
  struct capture file = {0};
  file.buf = malloc(MAX_OUT);
  read_file_into(&file, target);
  CHECK_U64(file.n_lines, 0);          /* nothing went through the link... */
  CHECK_U64(count(c, "end", "p1"), 1); /* ...and the events fell back to stderr */
  free(file.buf);
  unlink(link_path);
  unlink(target);
  free_capture(c);
}

static void test_plain_return_reports_the_real_status_where_glibc_allows(void) {
  struct capture *c = run_capture("plain_exit", NULL, NULL);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 7);
#if defined(__GLIBC__)
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "status"), 7);
#else
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "status"), 0); /* advisory: the wait status is authoritative */
#endif
  free_capture(c);
}

static void crash_reports(const char *scenario, int expected_signal, const char *id) {
  const char *on[] = {"HAIRSPRING_SIGNAL_REPORT=1", NULL};
  struct capture *c = run_capture(scenario, NULL, on);
  CHECK(WIFSIGNALED(c->status) && WTERMSIG(c->status) == expected_signal);
  check_all_lines_wellformed(c);
  CHECK(any_line_has(c, "\"reason\":\"signal\""));
  CHECK_U64(count(c, "end", "p1"), 1);
  CHECK_U64(field(find_line(c, "assert", "p1", id), "hits"), 1);
  free_capture(c);
}

static void test_real_faults_are_reported_when_handlers_are_on(void) {
#ifdef HS_T_ASAN
  printf("skip: SIGSEGV reports under AddressSanitizer\n");
#else
  crash_reports("segv", SIGSEGV, "segv.before");
  crash_reports("threadsegv", SIGSEGV, "threadsegv.worker");
#endif
  crash_reports("abort", SIGABRT, "abort.before");
}

static void test_report_survives_a_timer_signal_during_the_write(void) {
  struct capture *c = run_capture("alarm", NULL, NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "assert", "p1"), 64);
  CHECK_U64(count(c, "end", "p1"), 1);
  free_capture(c);
}

static void test_empty_catalog_reports_work_and_end(void) {
  struct capture *c = run_capture("empty", NULL, NULL);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "catalog", "p1"), 0);
  CHECK_U64(field(find_line(c, "end", "p1", NULL), "sites"), 0);
  CHECK_U64(field(find_line(c, "work", "p1", NULL), "ops"), 1);
  free_capture(c);
}

static void test_fork_while_another_thread_holds_the_key_lock_does_not_hang_the_child(void) {
  struct capture *c = run_capture("forklock", NULL, NULL);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0);
  free_capture(c);
}

static void test_a_lost_events_descriptor_is_dropped_not_redirected(void) {
  char path[] = "/tmp/hs_dup2_XXXXXX";
  int tmp = mkstemp(path);
  CHECK(tmp >= 0);
  close(tmp);
  struct capture *c = run_capture("dup2fd", path, NULL);
  struct capture data = {0};
  data.buf = malloc(MAX_OUT);
  read_file_into(&data, path);
  CHECK(!any_line_has(&data, "HAIRSPRING_EVENT")); /* the program's file never receives events */
  free(data.buf);
  unlink(path);
  free_capture(c);
}

static void eval_after_losing_the_descriptor(const char *const *env) {
  char path[] = "/tmp/hs_evalfd_XXXXXX";
  int tmp = mkstemp(path);
  CHECK(tmp >= 0);
  close(tmp);
  struct capture *c = run_capture("evalfd", path, env);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0);
  struct capture data = {0};
  data.buf = malloc(MAX_OUT);
  read_file_into(&data, path);
  CHECK(!any_line_has(&data, "HAIRSPRING_EVENT")); /* the program's file never receives an eval line */
  free(data.buf);
  unlink(path);
  free_capture(c);
}

static void test_eval_lines_check_the_events_descriptor_too(void) {
  const char *high[] = {"HS_EVALFD_MODE=high", NULL};
  eval_after_losing_the_descriptor(high);
  /* The fallback: with no room for a private copy the original stderr is the events descriptor. */
  const char *low[] = {"HS_EVALFD_MODE=low", "HS_TEST_NOFILE=10", NULL};
  eval_after_losing_the_descriptor(low);
}

static void test_an_ignored_signal_stays_ignored_and_is_reported_as_not_installed(void) {
  const char *env[] = {"HAIRSPRING_SIGNAL_REPORT=1", "HS_TEST_IGNORE_TERM=1", NULL};
  struct capture *c = run_capture("sigign", NULL, env);
  CHECK(WIFEXITED(c->status) && WEXITSTATUS(c->status) == 0); /* SIGTERM stayed ignored */
  check_all_lines_wellformed(c);
  CHECK(c->n_lines > 0 && has(c->lines[0], "\"signal_handlers\":false"));
  CHECK_U64(field(find_line(c, "assert", "p1", "sigign.after"), "hits"), 1);
  free_capture(c);
}

static void test_falling_back_to_the_original_descriptor_is_counted(void) {
  const char *env[] = {"HS_TEST_NOFILE=10", NULL}; /* no room for a descriptor numbered 20 or 200 */
  struct capture *c = run_capture("basic", NULL, env);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "end", "p1"), 1);
  CHECK(field(find_line(c, "end", "p1", NULL), "dropped") >= 1);
  free_capture(c);
}

static void test_reserved_and_malformed_root_tags_are_refused_not_shortened(void) {
  const char *bad_tags[] = {"HAIRSPRING_PROC=_eval", "HAIRSPRING_PROC=has space",
                            "HAIRSPRING_PROC=0123456789012345678901234567890123456789012345678901234567890123456789"};
  for (size_t i = 0; i < sizeof bad_tags / sizeof bad_tags[0]; i++) {
    const char *env[] = {bad_tags[i], NULL};
    struct capture *c = run_capture("basic", NULL, env);
    CHECK_U64(count(c, "end", "p1"), 1);
    CHECK(field(find_line(c, "end", "p1", NULL), "dropped") >= 1);
    free_capture(c);
  }
}

static void test_a_long_root_tag_gets_a_readable_hashed_child_tag(void) {
  /* 62 bytes: "<tag>.f1" would be 65, so the child gets the first 40 bytes, ".h" and a hash. */
  const char *env[] = {"HAIRSPRING_PROC=abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghij", NULL};
  struct capture *c = run_capture("fork", NULL, env);
  check_all_lines_wellformed(c);
  CHECK_U64(count(c, "end", "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghij"), 1);
  CHECK(any_line_has(c, "\"proc\":\"abcdefghijklmnopqrstuvwxyzabcdefghijklmn.h"));
  free_capture(c);
}

static void test_thread_lines_have_one_length_whatever_the_key(void) {
  struct capture *c = run_capture("threads", "12", NULL);
  size_t len = 0;
  for (size_t i = 0; i < c->n_lines; i++) {
    if (!is_kind(c->lines[i], "thread")) continue;
    if (len == 0) len = strlen(c->lines[i]);
    /* w-0..w-4 and t#1 differ in key length and stable/unstable in the flag; the line length must not. */
    CHECK_U64(strlen(c->lines[i]), len);
  }
  CHECK(len > 0);
  free_capture(c);
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: test_proc <sample-dir>\n");
    return 2;
  }
  g_dir = argv[1];
  RUN(test_basic_report_shape);
  RUN(test_same_id_at_two_call_sites_is_one_assert_line);
  RUN(test_fork_child_gets_derived_tag_and_fresh_counters);
  RUN(test_many_threads_each_get_a_thread_line);
  RUN(test_threads_beyond_the_table_are_capped_and_counted);
  RUN(test_signal_report_is_opt_in);
  RUN(test_exit_status_is_recorded_by_hs_exit);
  RUN(test_events_file_receives_the_report_instead_of_stderr);
  RUN(test_events_off_writes_nothing);
  RUN(test_eval_forms_write_one_line_each);
  RUN(test_two_processes_sharing_one_output_never_interleave_inside_a_line);
  RUN(test_events_survive_stderr_being_closed_and_reused);
  RUN(test_events_survive_stderr_being_closed_without_reuse);
  RUN(test_unwritable_events_file_falls_back_to_stderr_and_is_counted);
  RUN(test_events_file_symlink_is_not_followed);
  RUN(test_plain_return_reports_the_real_status_where_glibc_allows);
  RUN(test_real_faults_are_reported_when_handlers_are_on);
  RUN(test_report_survives_a_timer_signal_during_the_write);
  RUN(test_empty_catalog_reports_work_and_end);
  RUN(test_fork_while_another_thread_holds_the_key_lock_does_not_hang_the_child);
  RUN(test_a_lost_events_descriptor_is_dropped_not_redirected);
  RUN(test_eval_lines_check_the_events_descriptor_too);
  RUN(test_an_ignored_signal_stays_ignored_and_is_reported_as_not_installed);
  RUN(test_falling_back_to_the_original_descriptor_is_counted);
  RUN(test_reserved_and_malformed_root_tags_are_refused_not_shortened);
  RUN(test_a_long_root_tag_gets_a_readable_hashed_child_tag);
  RUN(test_thread_lines_have_one_length_whatever_the_key);
  return hs_t_summary();
}
