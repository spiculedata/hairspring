#!/bin/sh
# Runs build-record.sh for one variant in a linux/amd64 container, the build lane's environment for the record.
#
#   build-record-docker.sh <c|cxx-clang|cxx-gxx|go> [out-file]
#
# C and C++ use ubuntu:24.04 (gcc 13, g++ 13, clang 18, glibc 2.39: the box's toolchain); Go uses golang:1.27.3. Both
# images are pinned by digest. The source commit is passed into the container as SOURCE_COMMIT. The repository is mounted read-only except the fixtures directory, whose
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
# Pinned by digest (the multi-platform index digests, resolved with `docker buildx imagetools inspect`).
UBUNTU_IMAGE=ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac
GO_IMAGE=${GO_IMAGE:-golang:1.27.3@sha256:957861185dbf29d93668ace41e011d091c6e779953d47f0316ef2cc58e28fe2b}
SOURCE_COMMIT=${SOURCE_COMMIT:-$(git -C "$repo" rev-parse HEAD 2>/dev/null || echo unknown)}

case "$variant" in
  c | cxx-clang | cxx-gxx)
    image=$UBUNTU_IMAGE
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace gcc-13 g++-13 clang-18 >/dev/null'
    ;;
  go)
    image=$GO_IMAGE
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace >/dev/null'
    ;;
  *)
    echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
    exit 64
    ;;
esac

digest=$image
inside_out=/repo/examples/language-fixtures/build/record-$variant.md
docker run --rm --platform "$platform" --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
  -v "$repo/sdks:/repo/sdks:ro" -v "$repo/docs:/repo/docs:ro" -v "$fixtures:/repo/examples/language-fixtures" \
  -e IMAGE_DIGEST="$digest" -e SOURCE_COMMIT="$SOURCE_COMMIT" -e RUNS="${RUNS:-50}" \
  "$image" sh -c "$setup && sh /repo/examples/language-fixtures/scripts/build-record.sh $variant $inside_out"
rc=$?
if [ $# -ge 2 ] && [ -f "$fixtures/build/record-$variant.md" ]; then
  cp "$fixtures/build/record-$variant.md" "$2"
fi
exit $rc
