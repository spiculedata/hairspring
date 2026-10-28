#!/bin/sh
# Runs build-record.sh for one variant in a linux/amd64 container, the build lane's environment for the record.
#
#   build-record-docker.sh <c|cxx-clang|cxx-gxx|go|rust|rust-probe|java-probe|java|dotnet-probe> [out-file]
#
# C and C++ use ubuntu:24.04 (gcc 13, g++ 13, clang 18, glibc 2.39: the box's toolchain); Go uses golang:1.27.3; Rust
# uses rust:1.99.1-bookworm with the musl target of the platform (TARGET overrides it); the Java probe and the Java fixture
# use ubuntu:24.04 with
# the pinned Temurin 21.0.12.1+1 tarball, downloaded and checked against its sha256; the .NET probe uses the .NET SDK
# 10.0.402 image (runtime 10.0.13). The C, C++, Go, Ubuntu, Rust and .NET images are pinned by digest. The source commit
# is passed into the container as SOURCE_COMMIT. The repository is mounted read-only
# except the fixtures directory, whose build/ directory receives the artifacts and the record (Rust needs the whole
# workspace, so it mounts the whole repository read-only; its cargo output goes to a scratch directory). The container
# needs ptrace for strace (check B5) and an unconfined seccomp profile for the seccomp self-test (check B3); both are
# granted to this container only.
# This runs on the build machine. It never touches the box and never runs Hermit.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <c|cxx-clang|cxx-gxx|go|rust|rust-probe|java-probe|java|dotnet-probe> [out-file]" >&2
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
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"
# A tree with uncommitted changes is labelled +dirty: the record then does not name a commit that can be checked out.
SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}

# The pinned JDK of the Java probe: Temurin 21.0.12.1+1, the tarball and its published sha256.
JDK_URL=${JDK_URL:-https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jdk_x64_linux_hotspot_21.0.12.1_1.tar.gz}
JDK_SHA256=${JDK_SHA256:-ce79869e1307ed8ee1e2baa86a412b1eb5b75d10a01006d788a6f968bcfaee94}
extra_env=""
RUST_IMAGE=${RUST_IMAGE:-rust:1.99.1-bookworm@sha256:e12566d5abc17a0da741628782cdb504b68b8961f5bfcf677936bf3e8b32de29}
# The .NET SDK that ships runtime 10.0.13 (SDK 10.0.402), pinned by the image index digest.
DOTNET_IMAGE=${DOTNET_IMAGE:-mcr.microsoft.com/dotnet/sdk:10.0.402@sha256:747cbade5bfd918a175ac32f8f1d931edec2c7d0d5397a3de72eaef1ac1f66d2}
case "$platform" in
  linux/arm64) default_target=aarch64-unknown-linux-musl ;;
  *) default_target=x86_64-unknown-linux-musl ;;
esac
target=${TARGET:-$default_target}
# The .NET probe runs on the pinned runtime tarball (linux-x64) unless the container is not x86-64: then the installed one.
case "$platform" in
  linux/arm64) default_runtime_source=installed ;;
  *) default_runtime_source=pinned ;;
esac
mounts="-v $repo/global.json:/repo/global.json:ro -v $repo/sdks:/repo/sdks:ro -v $repo/docs:/repo/docs:ro -v $fixtures:/repo/examples/language-fixtures"
script="build-record.sh $variant"
inside_out=/repo/examples/language-fixtures/build/record-$variant.md
artifact_dir=/repo/examples/language-fixtures/build/artifacts

case "$variant" in
  c | cxx-clang | cxx-gxx)
    image=$UBUNTU_IMAGE
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace gcc-13 g++-13 clang-18 >/dev/null'
    ;;
  go)
    image=$GO_IMAGE
    setup='apt-get update -qq && apt-get install -y -qq make binutils strace >/dev/null'
    ;;
  rust | rust-probe)
    image=$RUST_IMAGE
    # Cargo needs the whole workspace, and the conformance tests build the C SDK inside its tree, so the container works
    # on a copy of the repository; only the fixtures' build directory comes back out.
    mkdir -p "$fixtures/build" || exit 1
    setup="cp -R /src /repo && apt-get update -qq && apt-get install -y -qq binutils strace file >/dev/null && rustup target add $target"
    mounts="-v $repo:/src:ro -v $fixtures/build:/out"
    script="build-record-rust.sh"
    [ "$variant" = rust-probe ] && script="build-record-probe-rust.sh"
    inside_out=/out/record-$variant.md
    artifact_dir=/out/artifacts
    ;;
  dotnet-probe)
    image=$DOTNET_IMAGE
    setup='apt-get update -qq && apt-get install -y -qq strace file >/dev/null'
    script="build-record-probe-dotnet.sh"
    ;;
  java-probe | java)
    # The pinned Temurin tarball is downloaded inside the container and its sha256 checked before it is extracted. The
    # java variant (the fixture with the Java SDK) skips the SDK's conformance tests, which need cargo: the CI job runs them.
    image=$UBUNTU_IMAGE
    setup="apt-get update -qq && apt-get install -y -qq make binutils strace unzip curl ca-certificates >/dev/null && curl -fsSL -o /tmp/jdk.tar.gz $JDK_URL && echo '$JDK_SHA256  /tmp/jdk.tar.gz' | sha256sum -c - && mkdir /opt/jdk && tar -xzf /tmp/jdk.tar.gz -C /opt/jdk --strip-components=1"
    script="build-record-probe-java.sh"
    [ "$variant" = java ] && script="build-record-java.sh"
    extra_env="-e JAVA_HOME=/opt/jdk"
    ;;
  *)
    echo "usage: $0 <c|cxx-clang|cxx-gxx|go|rust|rust-probe|java-probe|java|dotnet-probe> [out-file]" >&2
    exit 64
    ;;
esac

digest=$image
# The mounts and extra_env are separate words on purpose.
# shellcheck disable=SC2086
docker run --rm --platform "$platform" --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
  $mounts $extra_env \
  -e IMAGE_DIGEST="$digest" -e PRODUCER="docker $platform, image $image" -e SOURCE_COMMIT="$SOURCE_COMMIT" -e RUNS="${RUNS:-50}" \
  -e TARGET="$target" -e RUST_PINNED="${RUST_PINNED:-1.99.1}" -e ARTIFACT_DIR="$artifact_dir" \
  -e SDK_PINNED="${SDK_PINNED:-$(sh "$here/sdk-pin.sh")}" -e RUNTIME_PINNED="${RUNTIME_PINNED:-10.0.13}" \
  -e RUNTIME_SOURCE="${RUNTIME_SOURCE:-$default_runtime_source}" \
  "$image" sh -c "$setup && sh /repo/examples/language-fixtures/scripts/$script $inside_out"
rc=$?
if [ $# -ge 2 ] && [ -f "$fixtures/build/record-$variant.md" ]; then
  cp "$fixtures/build/record-$variant.md" "$2"
fi
exit $rc
