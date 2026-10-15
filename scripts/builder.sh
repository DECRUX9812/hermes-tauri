#!/usr/bin/env bash
# Run a command inside the Linux builder image (GTK/WebKitGTK -dev headers,
# cargo, tauri-cli). The current directory is mounted at the same path so
# paths in build output match the host. Serialized with the shared build lock.
#
#   scripts/builder.sh cargo tauri build --bundles deb,appimage
set -euo pipefail

IMAGE="${HERMES_TAURI_BUILDER:-hermes-tauri-builder:26.04}"
CACHE="${HERMES_TAURI_CACHE:-$HOME/.cache/hermes-tauri}"
mkdir -p "$CACHE/cargo" "$CACHE/target"

# Containers run in dockerd's cgroup, not the caller's, so cap memory here.
MEM="${HERMES_TAURI_BUILDER_MEM:-6g}"

# --init: without a PID 1 that reaps orphans, killed grandchildren stay zombies and process-group liveness checks never see them exit.
exec flock /tmp/hermes-build.lock docker run --rm --init \
  --memory "$MEM" --memory-swap "$MEM" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
  -e CARGO_HOME="$CACHE/cargo" \
  -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$CACHE/target}" \
  -e APPIMAGE_EXTRACT_AND_RUN=1 \
  -e NO_STRIP="${NO_STRIP:-}" \
  -v "$CACHE:$CACHE" \
  -v "$PWD:$PWD" -w "$PWD" \
  "$IMAGE" "$@"
