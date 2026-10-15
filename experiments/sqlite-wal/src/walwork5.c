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
static volatile int commits = 0;
static volatile int writers_done = 0;
static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;
static unsigned char *committed;  // [writer][txn]

static sqlite3 *open_db(void) {
  sqlite3 *db;
  if (sqlite3_open(db_path, &db) != SQLITE_OK) { fprintf(stderr, "open: %s\n", sqlite3_errmsg(db)); exit(3); }
  sqlite3_busy_timeout(db, 5000);
  char p[64]; snprintf(p, sizeof p, "PRAGMA wal_autocheckpoint=%d", autockpt); sqlite3_exec(db, p, 0, 0, 0);
  return db;
}

static void *writer(void *arg) {
  long id = (long)arg;
  sqlite3 *db = open_db();
  char sql[256];
  for (int i = 0; i < n_txns; i++) {
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

static int one_int(sqlite3 *db, const char *sql) {
  sqlite3_stmt *st; int v = -1;
  if (sqlite3_prepare_v2(db, sql, -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW) v = sqlite3_column_int(st, 0);
  sqlite3_finalize(st);
  return v;
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: walwork5 DB [txns writers autocheckpoint hold mode burst]\n"); return 2; }
  db_path = argv[1];
  if (argc > 2) n_txns = atoi(argv[2]);
  if (argc > 3) n_writers = atoi(argv[3]);
  if (argc > 4) autockpt = atoi(argv[4]);
  if (argc > 5) hold = atoi(argv[5]);
  if (argc > 6) mode = atoi(argv[6]);
  if (argc > 7) burst = atoi(argv[7]);
  if (argc > 5) hold = atoi(argv[5]);
  if (argc > 6) mode = atoi(argv[6]);
  if (argc > 7) burst = atoi(argv[7]);
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
  else pthread_create(&th[nt++], 0, ckpt_reader, 0);
  for (int t = 0; t < nt; t++) pthread_join(th[t], 0);

  db = open_db();
  int expect = 0; for (long k = 0; k < (long)n_txns * n_writers; k++) expect += committed[k];
  int counter = one_int(db, "SELECT v FROM z WHERE id=1");
  int rows = one_int(db, "SELECT count(*) FROM l");
  sqlite3_stmt *st; char integrity[256] = "?";
  if (sqlite3_prepare_v2(db, "PRAGMA integrity_check", -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW)
    snprintf(integrity, sizeof integrity, "%s", (const char *)sqlite3_column_text(st, 0));
  sqlite3_finalize(st);
  sqlite3_close(db);
  if (counter != expect || rows != expect) { printf("HAIRSPRING_VIOLATION: committed=%d counter=%d rows=%d\n", expect, counter, rows); return 1; }
  if (strcmp(integrity, "ok")) { printf("HAIRSPRING_VIOLATION: integrity_check: %.200s\n", integrity); return 1; }
  printf("ok sqlite %s committed=%d\n", sqlite3_libversion(), expect);
  return 0;
}
