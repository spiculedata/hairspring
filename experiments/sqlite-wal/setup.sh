#!/bin/sh
# Build the WAL-reset workload against SQLite 3.51.2 (affected) and 3.51.3 (fixed).
# Expects sqlite-amalgamation-3510200/ and sqlite-amalgamation-3510300/ (from sqlite.org/2026/) next to this file.
set -eu
cd "$(dirname "$0")"
for v in 3510200 3510300; do
  d=sqlite-amalgamation-$v
  gcc -O1 -pthread -I"$d" -o "walwork-$v" walwork.c "$d/sqlite3.c" -ldl -lm
  echo "native smoke $v:"; ./"walwork-$v" "smoke-$v.db" 3 100 1
done
