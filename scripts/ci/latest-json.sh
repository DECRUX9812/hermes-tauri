#!/usr/bin/env bash
# Generate Tauri v2 updater latest.json from signed artifacts.
# Reads artifacts from ./artifacts directory (populated by download-artifact step).
# Requires: jq (preinstalled on ubuntu runners), bash.
# Environment variables (provided by GitHub Actions):
#   GITHUB_REPOSITORY - e.g. "owner/repo"
#   GITHUB_REF_NAME   - e.g. "v0.1.0" (the tag name)
# Outputs: latest.json in the current working directory (uploaded as release asset).

set -euo pipefail

ARTIFACTS_DIR="${ARTIFACTS_DIR:-artifacts}"
REPO="${GITHUB_REPOSITORY:-}"
TAG="${GITHUB_REF_NAME:-}"

if [ -z "$REPO" ] || [ -z "$TAG" ]; then
  echo "::error::GITHUB_REPOSITORY and GITHUB_REF_NAME must be set"
  exit 1
fi

# Version is tag without leading 'v'
VERSION="${TAG#v}"

# Publication date in RFC3339 (UTC)
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Base download URL
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

# Find all .sig files
shopt -s nullglob
SIG_FILES=("$ARTIFACTS_DIR"/**/*.sig)
shopt -u nullglob

if [ ${#SIG_FILES[@]} -eq 0 ]; then
  echo "No .sig files found in $ARTIFACTS_DIR — skipping latest.json generation"
  exit 0
fi

echo "Found ${#SIG_FILES[@]} signature file(s), generating latest.json..."

# Build initial JSON
jq -n \
  --arg version "$VERSION" \
  --arg notes "" \
  --arg pub_date "$PUB_DATE" \
  '{
    version: $version,
    notes: $notes,
    pub_date: $pub_date,
    platforms: {}
  }' > latest.json.tmp

# We need to select the correct bundle per platform:
# linux-x86_64 / linux-aarch64 → the .AppImage (its .AppImage.sig)
# darwin-aarch64 / darwin-x86_64 → the .app.tar.gz (.app.tar.gz.sig)
# windows-x86_64 → the NSIS *-setup.exe (.exe.sig)
# Strategy: collect candidates per platform, then pick the preferred one.

declare -A platform_bundle
declare -A platform_sig
declare -A platform_priority

for sig in "${SIG_FILES[@]}"; do
  bundle="${sig%.sig}"
  [ -f "$bundle" ] || { echo "Warning: bundle file not found for $sig, skipping"; continue; }

  bundle_basename=$(basename "$bundle")
  platform=""
  priority=0

  case "$bundle_basename" in
    *.AppImage)
      case "$bundle_basename" in
        *x86_64*|*amd64*) platform="linux-x86_64"; priority=10 ;;
        *aarch64*|*arm64*) platform="linux-aarch64"; priority=10 ;;
      esac
      ;;
    *.app.tar.gz)
      case "$bundle_basename" in
        *aarch64*|*arm64*) platform="darwin-aarch64"; priority=10 ;;
        *x86_64*|*x64*) platform="darwin-x86_64"; priority=10 ;;
      esac
      ;;
    *_x64-setup.exe|*-x64-setup.exe)
      platform="windows-x86_64"
      priority=10
      ;;
    *.dmg)
      case "$bundle_basename" in
        *aarch64*|*arm64*) platform="darwin-aarch64"; priority=5 ;;
        *x86_64*|*x64*) platform="darwin-x86_64"; priority=5 ;;
      esac
      ;;
    *.deb|*.rpm)
      case "$bundle_basename" in
        *x86_64*|*amd64*) platform="linux-x86_64"; priority=5 ;;
        *aarch64*|*arm64*) platform="linux-aarch64"; priority=5 ;;
      esac
      ;;
    *.msi)
      case "$bundle_basename" in
        *x86_64*|*x64*) platform="windows-x86_64"; priority=5 ;;
      esac
      ;;
    *)
      # Fallback: infer from directory name
      dir_name=$(basename "$(dirname "$sig")")
      case "$dir_name" in
        *x86_64-unknown-linux-gnu*) platform="linux-x86_64"; priority=1 ;;
        *aarch64-unknown-linux-gnu*) platform="linux-aarch64"; priority=1 ;;
        *aarch64-apple-darwin*) platform="darwin-aarch64"; priority=1 ;;
        *x86_64-apple-darwin*) platform="darwin-x86_64"; priority=1 ;;
        *x86_64-pc-windows-msvc*) platform="windows-x86_64"; priority=1 ;;
      esac
      ;;
  esac

  [ -n "$platform" ] || { echo "Warning: could not determine platform for $bundle_basename, skipping"; continue; }

  # Keep the highest priority bundle for each platform
  current_priority=${platform_priority[$platform]:-0}
  if [ "$priority" -gt "$current_priority" ]; then
    platform_priority[$platform]=$priority
    platform_bundle[$platform]=$bundle_basename
    platform_sig[$platform]=$(cat "$sig" | tr -d '\n\r[:space:]')
    echo "Selected for $platform: $bundle_basename (priority $priority)"
  fi
done

# Now add selected platforms to JSON
for platform in "${!platform_bundle[@]}"; do
  bundle_basename="${platform_bundle[$platform]}"
  signature="${platform_sig[$platform]}"
  asset_url="${BASE_URL}/${bundle_basename}"

  jq \
    --arg platform "$platform" \
    --arg signature "$signature" \
    --arg url "$asset_url" \
    '.platforms[$platform] = {signature: $signature, url: $url}' \
    latest.json.tmp > latest.json.tmp2 && mv latest.json.tmp2 latest.json.tmp
done

# Finalize
mv latest.json.tmp latest.json
echo "Generated latest.json:"
cat latest.json