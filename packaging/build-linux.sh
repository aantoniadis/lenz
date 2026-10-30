#!/usr/bin/env bash
# Build the Linux release tarballs inside an Ubuntu 22.04 container, so the binaries need only glibc 2.35 rather than
# the build machine's newer one:
#   packaging/build-linux.sh 0.0.1            # x86_64 and aarch64
#   packaging/build-linux.sh 0.0.1 amd64      # one architecture
# aarch64 runs under qemu-user emulation (binfmt) and takes much longer. Output lands in target/package/ as with
# packaging/package.sh. Needs podman (or docker via ENGINE=docker).
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: packaging/build-linux.sh <version> [amd64|arm64]...}
shift
arches=("$@")
[ ${#arches[@]} -eq 0 ] && arches=(amd64 arm64)
engine=${ENGINE:-podman}
# An empty build context: the image needs no files, and the repository (with target/) would be sent whole.
ctx=$(mktemp -d)
trap 'rmdir "$ctx"' EXIT
for arch in "${arches[@]}"; do
    image="lenz-build:jammy-$arch"
    "$engine" build --platform "linux/$arch" -t "$image" -f - "$ctx" <<'EOF'
FROM docker.io/library/ubuntu:22.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl git pkg-config python3 libclang-dev \
    libxkbcommon-x11-dev libwayland-dev libfontconfig-dev libx11-xcb-dev libvulkan1 libglib2.0-dev libzstd-dev \
    && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
EOF
    # Separate target and registry caches per architecture; the source tree is mounted, not copied. The builder's
    # home is a distinct path (package.sh refuses binaries that name it; "/root" is also the tail of D-Bus paths).
    "$engine" run --rm --platform "linux/$arch" \
        -v "$PWD:/src" -w /src \
        -v "lenz-cargo-$arch:/root/.cargo/registry" \
        -e CARGO_TARGET_DIR="/src/target/container-$arch" \
        -e HOME=/home/builder -e RUSTUP_HOME=/root/.rustup -e CARGO_HOME=/root/.cargo \
        "$image" bash -c 'mkdir -p "$HOME" && git config --global --add safe.directory /src && packaging/package.sh "$0"' "$version"
done
ls -l target/package/*.tar.gz
