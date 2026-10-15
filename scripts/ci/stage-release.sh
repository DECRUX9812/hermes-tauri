#!/usr/bin/env bash
# Stage release artifacts and generate Tauri v2 updater latest.json.
# Reads artifacts from ./artifacts directory (populated by download-artifact step).
# Requires: jq (preinstalled on ubuntu runners), bash.
# Environment variables (provided by GitHub Actions):
#   GITHUB_REPOSITORY - e.g. "owner/repo"
#   GITHUB_REF_NAME   - e.g. "v0.1.0" (the tag name)
# Outputs: dist-release/ with staged files, and dist-release/latest.json (when .sig files exist).

set -euo pipefail

ARTIFACTS_DIR="${ARTIFACTS_DIR:-artifacts}"
REPO="${GITHUB_REPOSITORY:-}"
TAG="${GITHUB_REF_NAME:-}"

if [ -z "$REPO" ] || [ -z "$TAG" ]; then
  echo "::error::GITHUB_REPOSITORY and GITHUB_REF_NAME must be set"
  exit 1
fi

VERSION="${TAG#v}"
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

# Clean and create staging directory
rm -rf dist-release
mkdir -p dist-release

# Map from rust target (directory suffix) to platform key and preferred bundle pattern
declare -A TARGET_TO_PLATFORM=(
  ["x86_64-unknown-linux-gnu"]="linux-x86_64"
  ["aarch64-unknown-linux-gnu"]="linux-aarch64"
  ["aarch64-apple-darwin"]="darwin-aarch64"
  ["x86_64-apple-darwin"]="darwin-x86_64"
  ["x86_64-pc-windows-msvc"]="windows-x86_64"
)

declare -A PLATFORM_PREFERRED=(
  ["linux-x86_64"]="AppImage"
  ["linux-aarch64"]="AppImage"
  ["darwin-aarch64"]="app.tar.gz"
  ["darwin-x86_64"]="app.tar.gz"
  ["windows-x86_64"]="setup.exe"
)

# Find all bundle directories
shopt -s nullglob
BUNDLE_DIRS=("$ARTIFACTS_DIR"/bundle-*)
shopt -u nullglob

if [ ${#BUNDLE_DIRS[@]} -eq 0 ]; then
  echo "No bundle-* directories found in $ARTIFACTS_DIR"
  exit 1
fi

# First pass: collect all files with their platform, and determine updater bundles per platform
declare -a ALL_FILES=()
declare -a ALL_PLATFORMS=()
declare -A PLATFORM_BUNDLE=()      # Original bundle basename (for matching)
declare -A PLATFORM_SIG=()
declare -A PLATFORM_NEW_BUNDLE=()  # New deterministic name (for macOS updater bundle)

for bundle_dir in "${BUNDLE_DIRS[@]}"; do
  dir_name=$(basename "$bundle_dir")
  rust_target="${dir_name#bundle-}"
  platform="${TARGET_TO_PLATFORM[$rust_target]:-}"
  if [ -z "$platform" ]; then
    echo "Warning: unknown rust target $rust_target in $dir_name, skipping"
    continue
  fi

  # Find all files in this bundle directory (recursively)
  while IFS= read -r -d '' file; do
    [ -f "$file" ] || continue
    ALL_FILES+=("$file")
    ALL_PLATFORMS+=("$platform")
  done < <(find "$bundle_dir" -type f -print0)

  # Determine the preferred updater bundle for this platform
  preferred_pattern="${PLATFORM_PREFERRED[$platform]}"
  while IFS= read -r -d '' file; do
    [ -f "$file" ] || continue
    basename_file=$(basename "$file")
    [[ "$basename_file" == *.sig ]] && continue
    case "$platform" in
      linux-x86_64|linux-aarch64)
        [[ "$basename_file" == *.AppImage ]] || continue
        ;;
      darwin-aarch64|darwin-x86_64)
        [[ "$basename_file" == *.app.tar.gz ]] || continue
        ;;
      windows-x86_64)
        [[ "$basename_file" == *setup.exe ]] || continue
        ;;
    esac
    sig_file="${file}.sig"
    if [ -f "$sig_file" ]; then
      orig_bundle=$(basename "$file")
      PLATFORM_BUNDLE[$platform]=$orig_bundle
      PLATFORM_SIG[$platform]=$(cat "$sig_file" | tr -d '\n\r[:space:]')
      # Pre-compute the new deterministic name for macOS
      if [[ "$platform" == darwin-* ]]; then
        arch="${platform#darwin-}"
        PLATFORM_NEW_BUNDLE[$platform]="Hermes-Tauri_${VERSION}_${arch}.app.tar.gz"
      fi
      echo "Selected updater bundle for $platform: $orig_bundle"
    fi
    break
  done < <(find "$bundle_dir" -type f -print0)
done

# Compute deterministic staged names for all files, and track staged name per platform
declare -a STAGE_FILES=()
declare -a STAGE_DESTS=()
declare -A PLATFORM_STAGED_BUNDLE=()

for i in "${!ALL_FILES[@]}"; do
  file="${ALL_FILES[$i]}"
  platform="${ALL_PLATFORMS[$i]}"
  basename_file=$(basename "$file")
  staged_name="${basename_file// /-}"
  
  # Special handling: macOS updater bundle and its .sig get deterministic names per platform
  if [[ "$platform" == darwin-* ]]; then
    orig_bundle="${PLATFORM_BUNDLE[$platform]:-}"
    new_bundle="${PLATFORM_NEW_BUNDLE[$platform]:-}"
    if [ -n "$orig_bundle" ] && [ -n "$new_bundle" ] && ( [ "$basename_file" = "$orig_bundle" ] || [ "$basename_file" = "${orig_bundle}.sig" ] ); then
      if [ "$basename_file" = "$orig_bundle" ]; then
        staged_name="$new_bundle"
        PLATFORM_STAGED_BUNDLE[$platform]="$new_bundle"
      else
        staged_name="${new_bundle}.sig"
      fi
    else
      # For non-updater macOS files (e.g., .dmg), use space-replaced name
      if [ -z "${PLATFORM_STAGED_BUNDLE[$platform]:-}" ] && [ "$basename_file" = "${PLATFORM_BUNDLE[$platform]:-}" ]; then
        PLATFORM_STAGED_BUNDLE[$platform]="$staged_name"
      fi
    fi
  else
    # For non-macOS platforms, the staged name is the space-replaced original
    if [ -z "${PLATFORM_STAGED_BUNDLE[$platform]:-}" ] && [ "$basename_file" = "${PLATFORM_BUNDLE[$platform]:-}" ]; then
      PLATFORM_STAGED_BUNDLE[$platform]="$staged_name"
    fi
  fi
  
  STAGE_FILES+=("$file")
  STAGE_DESTS+=("dist-release/$staged_name")
done

# Check for collisions in staged names
declare -A DEST_COUNT=()
for dest in "${STAGE_DESTS[@]}"; do
  DEST_COUNT["$dest"]=$((${DEST_COUNT["$dest"]:-0} + 1))
done

for dest in "${!DEST_COUNT[@]}"; do
  if [ "${DEST_COUNT[$dest]}" -gt 1 ]; then
    echo "::error::Collision in staged names: $dest would be written ${DEST_COUNT[$dest]} times"
    exit 1
  fi
done

# Copy files to staging directory
for i in "${!STAGE_FILES[@]}"; do
  src="${STAGE_FILES[$i]}"
  dest="${STAGE_DESTS[$i]}"
  cp "$src" "$dest"
done

# If no signatures found, exit 0 (unsigned build) - no latest.json
sig_count=0
for platform in "${!PLATFORM_SIG[@]}"; do
  if [ -n "${PLATFORM_SIG[$platform]:-}" ]; then
    sig_count=$((sig_count + 1))
  fi
done

if [ $sig_count -eq 0 ]; then
  echo "No .sig files found — skipping latest.json generation (unsigned build)"
  ls -la dist-release/
  exit 0
fi

# Generate latest.json using STAGED names
jq -n \
  --arg version "$VERSION" \
  --arg notes "" \
  --arg pub_date "$PUB_DATE" \
  '{
    version: $version,
    notes: $notes,
    pub_date: $pub_date,
    platforms: {}
  }' > dist-release/latest.json.tmp

for platform in "${!PLATFORM_SIG[@]}"; do
  signature="${PLATFORM_SIG[$platform]}"
  [ -n "$signature" ] || continue
  
  # Use the staged bundle name for this platform
  bundle_basename="${PLATFORM_STAGED_BUNDLE[$platform]}"
  [ -n "$bundle_basename" ] || continue
  
  # URL-encode the basename using jq's @uri
  asset_url="${BASE_URL}/$(printf '%s' "$bundle_basename" | jq -sRr @uri)"
  
  jq \
    --arg platform "$platform" \
    --arg signature "$signature" \
    --arg url "$asset_url" \
    '.platforms[$platform] = {signature: $signature, url: $url}' \
    dist-release/latest.json.tmp > dist-release/latest.json.tmp2 && mv dist-release/latest.json.tmp2 dist-release/latest.json.tmp
done

mv dist-release/latest.json.tmp dist-release/latest.json

echo "Generated dist-release/latest.json:"
cat dist-release/latest.json

echo "Staged files:"
ls -la dist-release/

# Verify each URL's basename exists in dist-release
echo "Verifying URLs match staged files..."
jq -r '.platforms[] | .url' dist-release/latest.json | while IFS= read -r url; do
  basename_url=$(basename "$url")
  # URL decode for filesystem check
  decoded=$(printf '%b' "${basename_url//%/\\x}")
  if [ ! -f "dist-release/$decoded" ]; then
    echo "::error::URL basename $decoded not found in dist-release/"
    exit 1
  fi
done

echo "All verifications passed."