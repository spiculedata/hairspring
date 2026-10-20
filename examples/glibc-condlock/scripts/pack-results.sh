#!/bin/sh
# pack-results.sh <results dir>: packs each cell's raw run files into raw/<cell>.tar.gz (the raw directories are kept;
# only the tarballs are meant to be committed) and records their sha256 in TARBALLS.sha256. Refuses to run twice.
set -eu

res=${1:?usage: pack-results.sh <results dir>}
[ -d "$res" ] || { echo "pack-results.sh: no such directory $res" >&2; exit 2; }
if [ -e "$res/raw" ]; then
  echo "pack-results.sh: refusing to overwrite existing $res/raw" >&2
  exit 1
fi
mkdir "$res/raw"
for d in "$res"/*/; do
  cell=$(basename "$d")
  [ "$cell" = raw ] && continue
  tar --uname owner --gname group --numeric-owner -C "$res" -czf "$res/raw/$cell.tar.gz" "$cell"
done
(cd "$res" && shasum -a 256 raw/*.tar.gz >TARBALLS.sha256)
{
  echo "tarballs (member count = entries listed by tar -tzf, directory entry included):"
  for t in "$res"/raw/*.tar.gz; do
    echo "  raw/$(basename "$t"): $(tar -tzf "$t" | wc -l | tr -d ' ') members"
  done
} >>"$res/MANIFEST"
echo "packed: $(wc -l <"$res/TARBALLS.sha256" | tr -d ' ') tarball(s) in $res/raw"
