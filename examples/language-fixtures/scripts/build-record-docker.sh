#!/bin/sh
# Runs build-record.sh for one variant in a linux/amd64 container, the build lane's environment for the record.
#
#   build-record-docker.sh <c|cxx-clang|cxx-gxx|go> [out-file]
#
# C and C++ use ubuntu:24.04 (gcc 13, g++ 13, clang 18, glibc 2.39: the box's toolchain); Go uses the golang image
# named by GO_IMAGE (default golang:1.27). The repository is mounted read-only except the fixtures directory, whose
# build/ directory receives the artifacts and the record. The container needs ptrace for strace (check B5) and an
# unconfined seccomp profile for the seccomp self-test (check B3); both are granted to this container only.
# This runs on the build machine. It never touches the box and never runs Hermit.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
  exit 64
fi
variant=$1
here=$(cd "$(dirname "$0")" && pwd)
fixtures=$(dirname "$here")
repo=$(cd "$fixtures/../.." && pwd)
platform=${PLATFORM:-linux/amd64}

case "$variant" in
  c | cxx-clang | cxx-gxx)
    image=ubuntu:24.04
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace gcc-13 g++-13 clang-18 >/dev/null'
    ;;
  go)
    image=${GO_IMAGE:-golang:1.27}
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace >/dev/null'
    ;;
  *)
    echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
    exit 64
    ;;
esac

digest=$(docker image inspect --format '{{index .RepoDigests 0}}' "$image" 2>/dev/null || echo "$image (digest unknown)")
inside_out=/repo/examples/language-fixtures/build/record-$variant.md
docker run --rm --platform "$platform" --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
  -v "$repo/sdks:/repo/sdks:ro" -v "$repo/docs:/repo/docs:ro" -v "$fixtures:/repo/examples/language-fixtures" \
  -e IMAGE_DIGEST="$digest" -e RUNS="${RUNS:-50}" \
  "$image" sh -c "$setup && sh /repo/examples/language-fixtures/scripts/build-record.sh $variant $inside_out"
rc=$?
if [ $# -ge 2 ] && [ -f "$fixtures/build/record-$variant.md" ]; then
  cp "$fixtures/build/record-$variant.md" "$2"
fi
exit $rc
