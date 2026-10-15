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

exec flock /tmp/hermes-build.lock docker run --rm \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e CARGO_HOME="$CACHE/cargo" \
  -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$CACHE/target}" \
  -e APPIMAGE_EXTRACT_AND_RUN=1 \
  -e NO_STRIP="${NO_STRIP:-}" \
  -v "$CACHE:$CACHE" \
  -v "$PWD:$PWD" -w "$PWD" \
  "$IMAGE" "$@"
