#!/bin/sh
# Download and verify the SQLite 3.51.2 (affected) and 3.51.3 (fixed) amalgamations, then build the baseline workload.
# Amalgamations are NOT vendored in this repository.
set -eu
cd "$(dirname "$0")"
fetch() {  # version, expected sha256
  v=$1; sum=$2; z=s$v.zip
  [ -f "$z" ] || curl -fsSL -o "$z" "https://sqlite.org/2026/sqlite-amalgamation-$v.zip"
  echo "$sum  $z" | sha256sum -c -
  python3 -c "import zipfile;zipfile.ZipFile('$z').extractall('.')"
}
fetch 3510200 6e2a845a493026bdbad0618b2b5a0cf48584faab47384480ed9f592d912f23ec
fetch 3510300 acb1e6f5d832484bf6d32b681e858c38add8b2acdfd42ac5df24b8afb46552b4
for v in 3510200 3510300; do
  d=sqlite-amalgamation-$v
  gcc -O1 -pthread -I"$d" -o "walwork9-$v" src/walwork9.c "$d/sqlite3.c" -ldl -lm
  echo "native smoke $v:"; ./"walwork9-$v" "smoke-$v.db" 600 1 0 5 4 0 3 1 1
done
