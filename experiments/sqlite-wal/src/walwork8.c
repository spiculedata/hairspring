// Workload v2 for the SQLite WAL-reset bug. Shape: one thread owns a checkpointer connection and a reader
// connection; another thread owns a writer connection doing tiny single-row transactions so the WAL keeps
// getting reset. Property: every committed write is present and integrity_check is ok.
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "sqlite3.h"

#define MAX_TXNS 200000
static const char *db_path;
static int n_txns = 2000, n_writers = 1, autockpt = 0, hold = 10, mode = 1, burst = 4;
static volatile int commits = 0, pause_req = 0, paused = 0, cycles = 0, parked = 0, sync_off = 0, group = 3;
static volatile int writers_done = 0;
static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;
static unsigned char *committed;  // [writer][txn]

static sqlite3 *open_db(void) {
  sqlite3 *db;
  if (sqlite3_open(db_path, &db) != SQLITE_OK) { fprintf(stderr, "open: %s\n", sqlite3_errmsg(db)); exit(3); }
  sqlite3_busy_timeout(db, 5000);
  char p[64]; snprintf(p, sizeof p, "PRAGMA wal_autocheckpoint=%d", autockpt); sqlite3_exec(db, p, 0, 0, 0);
  if (sync_off) sqlite3_exec(db, "PRAGMA synchronous=OFF", 0, 0, 0);
  return db;
}

static void *writer(void *arg) {
  long id = (long)arg;
  sqlite3 *db = open_db();
  char sql[256];
  for (int i = 0; i < n_txns; i++) {
    if (mode == 3) { while (pause_req) { paused = 1; sched_yield(); } paused = 0; }
    if (mode == 4 && i > 0 && i % group == 0) { parked = 1; while (parked && writers_done < n_writers) sched_yield(); }
    snprintf(sql, sizeof sql, "BEGIN; UPDATE z SET v=v+1 WHERE id=1; INSERT INTO l(w,i) VALUES(%ld,%d); COMMIT;", id, i);
    if (sqlite3_exec(db, sql, 0, 0, 0) == SQLITE_OK) { committed[id * n_txns + i] = 1; __sync_fetch_and_add(&commits, 1); }
    else sqlite3_exec(db, "ROLLBACK", 0, 0, 0);
    sched_yield();
  }
  sqlite3_close(db);
  pthread_mutex_lock(&mu); writers_done++; pthread_mutex_unlock(&mu);
  return 0;
}

static void hold_reader(sqlite3 *c) {
  sqlite3_exec(c, "BEGIN; SELECT v FROM z", 0, 0, 0);
  int target = commits + hold;
  while (commits < target && writers_done < n_writers) sched_yield();
  sqlite3_exec(c, "COMMIT", 0, 0, 0);
}

// mode 1 (v4a, diagnostic): one thread; after each reader hold, a burst of PASSIVE checkpoints. The first backfills
// fully; the rest START on a fully backfilled WAL, which is the state the stale-header check needs.
static void *ckpt_reader(void *arg) {
  (void)arg;
  sqlite3 *a = open_db(), *c = open_db();
  for (long k = 0; k < (long)n_txns * n_writers && writers_done < n_writers; k++) {
    hold_reader(c);
    for (int r = 0; r < burst; r++) { sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0); sched_yield(); }
  }
  sqlite3_close(a); sqlite3_close(c);
  return 0;
}

// mode 2 (v4b, generic shape): reader-holder and a free-running (bounded) PASSIVE checkpointer on separate threads.
static void *holder(void *arg) {
  (void)arg;
  sqlite3 *c = open_db();
  for (long k = 0; k < (long)n_txns * n_writers && writers_done < n_writers; k++) { hold_reader(c); sched_yield(); }
  sqlite3_close(c);
  return 0;
}

static void *ckpt_free(void *arg) {
  (void)arg;
  sqlite3 *a = open_db();
  for (long k = 0; k < 4L * n_txns * n_writers && writers_done < n_writers; k++) {
    sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0);
    sched_yield();
  }
  sqlite3_close(a);
  return 0;
}

// mode 3 (v5, gated): per cycle, the reader pins a snapshot while the writer runs `hold` commits; then the writer is
// PAUSED, checkpoint 1 backfills everything (generation length ~hold), the writer is released immediately before
// checkpoint 2, so checkpoint 2's header read races the writer's WAL restart on a fully backfilled WAL.
static void *ckpt_gated(void *arg) {
  (void)arg;
  sqlite3 *a = open_db(), *c = open_db();
  for (long k = 0; k < (long)n_txns * n_writers && writers_done < n_writers; k++) {
    hold_reader(c);
    pause_req = 1;
    while (!paused && writers_done < n_writers) sched_yield();
    sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0);
    pause_req = 0;
    sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0);
    cycles++;
  }
  pause_req = 0;
  sqlite3_close(a); sqlite3_close(c);
  return 0;
}

// mode 4 (strict alternation): the writer commits exactly `group` transactions (WAL grows to M0 frames, nothing
// checkpoints it: autocheckpoint is off) and parks. ckpt 1 backfills everything; the writer is released immediately
// before ckpt 2, which races the writer's WAL restart on a fully backfilled WAL. No reader thread, no wasted commits.
static void *ckpt_alternate(void *arg) {
  (void)arg;
  sqlite3 *a = open_db();
  while (writers_done < n_writers) {
    while (!parked && writers_done < n_writers) sched_yield();
    if (writers_done >= n_writers) break;
    sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0);
    parked = 0;
    sqlite3_exec(a, "PRAGMA wal_checkpoint(PASSIVE)", 0, 0, 0);
    cycles++;
  }
  parked = 0;
  sqlite3_close(a);
  return 0;
}

static int one_int(sqlite3 *db, const char *sql) {
  sqlite3_stmt *st; int v = -1;
  if (sqlite3_prepare_v2(db, sql, -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW) v = sqlite3_column_int(st, 0);
  sqlite3_finalize(st);
  return v;
}

// Remove the database files on every exit path so a second run in the same /tmp (as under `hermit run --verify`)
// starts clean and the two runs make identical syscalls.
static void cleanup_files(void) {
  char tmp[512];
  unlink(db_path);
  snprintf(tmp, sizeof tmp, "%s-wal", db_path); unlink(tmp);
  snprintf(tmp, sizeof tmp, "%s-shm", db_path); unlink(tmp);
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: walwork7 DB [txns writers autocheckpoint hold mode burst group sync_off]\n"); return 2; }
  db_path = argv[1];
  atexit(cleanup_files);
  if (argc > 2) n_txns = atoi(argv[2]);
  if (argc > 3) n_writers = atoi(argv[3]);
  if (argc > 4) autockpt = atoi(argv[4]);
  if (argc > 5) hold = atoi(argv[5]);
  if (argc > 6) mode = atoi(argv[6]);
  if (argc > 7) burst = atoi(argv[7]);
  if (argc > 8) group = atoi(argv[8]);
  if (argc > 9) sync_off = atoi(argv[9]);
  if (n_txns < 1 || n_writers < 1 || n_writers > 8 || (long)n_txns * n_writers > MAX_TXNS) return 2;
  committed = calloc((size_t)n_txns * n_writers, 1);
  char tmp[512];
  unlink(db_path);
  snprintf(tmp, sizeof tmp, "%s-wal", db_path); unlink(tmp);
  snprintf(tmp, sizeof tmp, "%s-shm", db_path); unlink(tmp);
  sqlite3 *db = open_db();
  sqlite3_exec(db, "PRAGMA journal_mode=WAL; CREATE TABLE z(id INTEGER PRIMARY KEY, v INT); INSERT INTO z VALUES(1,0); CREATE TABLE l(w INT, i INT);", 0, 0, 0);
  sqlite3_close(db);
  pthread_t th[16]; int nt = 0;
  for (long w = 0; w < n_writers; w++) pthread_create(&th[nt++], 0, writer, (void *)w);
  if (mode == 2) { pthread_create(&th[nt++], 0, holder, 0); pthread_create(&th[nt++], 0, ckpt_free, 0); }
  else if (mode == 3) pthread_create(&th[nt++], 0, ckpt_gated, 0);
  else if (mode == 4) pthread_create(&th[nt++], 0, ckpt_alternate, 0);
  else pthread_create(&th[nt++], 0, ckpt_reader, 0);
  for (int t = 0; t < nt; t++) pthread_join(th[t], 0);

  db = open_db();
  int expect = 0; for (long k = 0; k < (long)n_txns * n_writers; k++) expect += committed[k];
  int counter = one_int(db, "SELECT v FROM z WHERE id=1");
  int rows = one_int(db, "SELECT count(*) FROM l");
  char rowsErr[128] = ""; if (rows == -1) snprintf(rowsErr, sizeof rowsErr, "%s", sqlite3_errmsg(db));
  sqlite3_stmt *st; char integrity[256] = "?";
  if (sqlite3_prepare_v2(db, "PRAGMA integrity_check", -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW)
    snprintf(integrity, sizeof integrity, "%s", (const char *)sqlite3_column_text(st, 0));
  sqlite3_finalize(st);
  sqlite3_close(db);
  if (counter != expect || rows != expect) { printf("HAIRSPRING_VIOLATION: committed=%d counter=%d rows=%d cycles=%d integrity=%.60s err=%.60s\n", expect, counter, rows, cycles, integrity, rowsErr); return 1; }
  if (strcmp(integrity, "ok")) { printf("HAIRSPRING_VIOLATION: integrity_check: %.200s\n", integrity); return 1; }
  printf("ok sqlite %s committed=%d cycles=%d\n", sqlite3_libversion(), expect, cycles);
  return 0;
}
