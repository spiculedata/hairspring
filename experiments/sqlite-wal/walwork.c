// Workload for the SQLite WAL-reset bug: concurrent writers and checkpointers on one WAL database,
// then verify no committed write was lost and the database is not corrupt.
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "sqlite3.h"

#define MAX_WRITERS 16
#define MAX_TXNS 4096
#define MAX_CKPTS 400
static const char *db_path;
static int n_writers = 3, n_txns = 100, n_ckpt = 1;
static int writers_done = 0;
static pthread_mutex_t done_mu = PTHREAD_MUTEX_INITIALIZER;
static unsigned char committed[MAX_WRITERS][MAX_TXNS];

static sqlite3 *open_db(void) {
  sqlite3 *db;
  if (sqlite3_open(db_path, &db) != SQLITE_OK) { fprintf(stderr, "open: %s\n", sqlite3_errmsg(db)); exit(3); }
  sqlite3_busy_timeout(db, 5000);
  return db;
}

static void *writer(void *arg) {
  long id = (long)arg;
  sqlite3 *db = open_db();
  char sql[128];
  for (int i = 0; i < n_txns; i++) {
    snprintf(sql, sizeof sql, "INSERT INTO t(w,i,pad) VALUES(%ld,%d,zeroblob(300))", id, i);
    if (sqlite3_exec(db, sql, 0, 0, 0) == SQLITE_OK) committed[id][i] = 1;  // autocommit: ok means committed
  }
  sqlite3_close(db);
  pthread_mutex_lock(&done_mu); writers_done++; pthread_mutex_unlock(&done_mu);
  return 0;
}

static void *checkpointer(void *arg) {
  long id = (long)arg;
  sqlite3 *db = open_db();
  const char *modes[] = {"PASSIVE", "RESTART", "TRUNCATE", "FULL"};
  char sql[64];
  for (long k = id;; k++) {
    pthread_mutex_lock(&done_mu); int done = writers_done; pthread_mutex_unlock(&done_mu);
    if (done >= n_writers) break;
    snprintf(sql, sizeof sql, "PRAGMA wal_checkpoint(%s)", modes[k % 4]);
    sqlite3_exec(db, sql, 0, 0, 0);
    if (k - id >= MAX_CKPTS) break;  // bounded: a spinning checkpointer can starve writers under a deterministic scheduler
    sched_yield();
  }
  sqlite3_close(db);
  return 0;
}

static int one_int(sqlite3 *db, const char *sql) {
  sqlite3_stmt *st; int v = -1;
  if (sqlite3_prepare_v2(db, sql, -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW) v = sqlite3_column_int(st, 0);
  sqlite3_finalize(st);
  return v;
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: walwork DB [writers txns checkpointers]\n"); return 2; }
  db_path = argv[1];
  if (argc > 2) n_writers = atoi(argv[2]);
  if (argc > 3) n_txns = atoi(argv[3]);
  if (argc > 4) n_ckpt = atoi(argv[4]);
  if (n_writers < 1 || n_writers > MAX_WRITERS || n_txns < 1 || n_txns > MAX_TXNS) return 2;
  char tmp[512];
  unlink(db_path);
  snprintf(tmp, sizeof tmp, "%s-wal", db_path); unlink(tmp);
  snprintf(tmp, sizeof tmp, "%s-shm", db_path); unlink(tmp);
  sqlite3 *db = open_db();
  sqlite3_exec(db, "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=8; CREATE TABLE t(w INT, i INT, pad BLOB);", 0, 0, 0);
  sqlite3_close(db);
  pthread_t th[MAX_WRITERS + 8];
  int nt = 0;
  for (long w = 0; w < n_writers; w++) pthread_create(&th[nt++], 0, writer, (void *)w);
  for (long c = 0; c < n_ckpt && c < 8; c++) pthread_create(&th[nt++], 0, checkpointer, (void *)c);
  for (int t = 0; t < nt; t++) pthread_join(th[t], 0);

  db = open_db();
  int missing = 0, first_w = -1, first_i = -1;
  char sql[128];
  for (int w = 0; w < n_writers; w++)
    for (int i = 0; i < n_txns; i++) {
      if (!committed[w][i]) continue;
      snprintf(sql, sizeof sql, "SELECT count(*) FROM t WHERE w=%d AND i=%d", w, i);
      if (one_int(db, sql) != 1) { if (!missing++) { first_w = w; first_i = i; } }
    }
  sqlite3_stmt *st; char integrity[256] = "?";
  if (sqlite3_prepare_v2(db, "PRAGMA integrity_check", -1, &st, 0) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW)
    snprintf(integrity, sizeof integrity, "%s", (const char *)sqlite3_column_text(st, 0));
  sqlite3_finalize(st);
  sqlite3_close(db);
  if (missing) { printf("HAIRSPRING_VIOLATION: %d committed writes lost (first: writer %d txn %d)\n", missing, first_w, first_i); return 1; }
  if (strcmp(integrity, "ok")) { printf("HAIRSPRING_VIOLATION: integrity_check: %s\n", integrity); return 1; }
  printf("ok sqlite %s\n", sqlite3_libversion());
  return 0;
}
