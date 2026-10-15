# CI/CD Documentation

## Overview

This document describes the GitHub Actions workflow for building the Hermes Tauri application across multiple platforms.

## Workflow: `.github/workflows/build.yml`

### Top-Level Configuration
- **Workflow Name**: `Build`
- **Permissions**: `contents: read` (scoped globally, elevated to `contents: write` only within the release job)
- **Triggers**:
  - `push` to `master`
  - Tag creation matching `v*`
  - `pull_request` against all branches
  - `workflow_dispatch` (manual trigger)

---

### Jobs

#### 1. `check` (`ubuntu-22.04`)
Runs code quality and verification checks on every push, PR, and tag:
- **System Dependencies**: Installs Tauri Linux build libraries via `apt-get`:
  - `libwebkit2gtk-4.1-dev`
  - `libgtk-3-dev`
  - `libayatana-appindicator3-dev`
  - `librsvg2-dev`
  - `libxdo-dev`
  - `libssl-dev`
  - `patchelf`
- **Rust Toolchain**: `dtolnay/rust-toolchain@stable` with components `rustfmt` and `clippy`.
- **Caching**: `Swatinem/rust-cache@v2`.
- **Quality Checks**:
  - Formatting: `cargo fmt --all -- --check`
  - Linting: `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - Test Suite: `cargo test --workspace --locked`

Clippy, the tests, and the release build pass `--locked`, so CI builds from the committed `Cargo.lock` and fails instead of re-resolving it.

---

#### 2. `build` (Matrix, `needs: check`)
Compiles the application across the target operating system matrix.
- **Fail-Fast**: Disabled (`fail-fast: false`), ensuring that a failure on one target does not cancel builds on other platforms.
- **Tauri Action**: Uses `tauri-apps/tauri-action@v0` with `tauriScript: cargo tauri`.

##### Matrix Configuration

| Platform | Runner OS | Target Triple | Configured Bundles |
|----------|-----------|---------------|--------------------|
| Linux x86_64 | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `deb,appimage,rpm` |
| Linux ARM64 | `ubuntu-22.04-arm` | `aarch64-unknown-linux-gnu` | `deb,appimage` |
| macOS ARM64 | `macos-14` | `aarch64-apple-darwin` | `app,dmg` |
| macOS x64 (Cross) | `macos-14` | `x86_64-apple-darwin` | `app,dmg` |
| Windows x64 | `windows-latest` | `x86_64-pc-windows-msvc` | `nsis,msi` |

##### Signing Environment Preparation
The `Prepare signing environment` step selectively exports signing secrets to `$GITHUB_ENV` to prevent empty variables from causing `tauri-action` to fail:
- Single-line secrets are written only when set and non-empty:
  - `APPLE_CERTIFICATE_PASSWORD`
  - `APPLE_SIGNING_IDENTITY`
  - `APPLE_ID`
  - `APPLE_PASSWORD`
  - `APPLE_TEAM_ID`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- Multiline secrets (`APPLE_CERTIFICATE`, `TAURI_SIGNING_PRIVATE_KEY`) are exported using a heredoc with a randomized boundary delimiter (`EOF_$(openssl rand -hex 8)`), preventing payload text from truncating the block.
- When `TAURI_SIGNING_PRIVATE_KEY` is present and non-empty, the step generates an updater configuration snippet at `$RUNNER_TEMP/updater.conf.json` with `{"bundle":{"createUpdaterArtifacts":true}}` and passes `--config $RUNNER_TEMP/updater.conf.json` to the build args. When unset, no updater artifacts or `.sig` signatures are generated.

##### Artifact Uploads
Each matrix runner uploads its deliverable outputs using `actions/upload-artifact@v4` under a unique artifact name `bundle-${{ matrix.target }}` with 7-day retention (`retention-days: 7`) and `if-no-files-found: error`.

Explicit globs are used so that intermediate packaging directories (such as `AppDir/`, `.app/Contents/`, or `DEBIAN/`) are excluded:

| Matrix Target | Upload Artifact Name | Uploaded Deliverable Files |
|---------------|----------------------|----------------------------|
| `x86_64-unknown-linux-gnu` | `bundle-x86_64-unknown-linux-gnu` | `target/x86_64-unknown-linux-gnu/release/bundle/deb/*.deb`<br>`target/x86_64-unknown-linux-gnu/release/bundle/rpm/*.rpm`<br>`target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage`<br>`target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage.sig` (when signed) |
| `aarch64-unknown-linux-gnu` | `bundle-aarch64-unknown-linux-gnu` | `target/aarch64-unknown-linux-gnu/release/bundle/deb/*.deb`<br>`target/aarch64-unknown-linux-gnu/release/bundle/appimage/*.AppImage`<br>`target/aarch64-unknown-linux-gnu/release/bundle/appimage/*.AppImage.sig` (when signed) |
| `aarch64-apple-darwin` | `bundle-aarch64-apple-darwin` | `target/aarch64-apple-darwin/release/bundle/dmg/*.dmg`<br>`target/aarch64-apple-darwin/release/bundle/macos/*.app.tar.gz`<br>`target/aarch64-apple-darwin/release/bundle/macos/*.app.tar.gz.sig` (when signed) |
| `x86_64-apple-darwin` | `bundle-x86_64-apple-darwin` | `target/x86_64-apple-darwin/release/bundle/dmg/*.dmg`<br>`target/x86_64-apple-darwin/release/bundle/macos/*.app.tar.gz`<br>`target/x86_64-apple-darwin/release/bundle/macos/*.app.tar.gz.sig` (when signed) |
| `x86_64-pc-windows-msvc` | `bundle-x86_64-pc-windows-msvc` | `target/x86_64-pc-windows-msvc/release/bundle/nsis/*-setup.exe`<br>`target/x86_64-pc-windows-msvc/release/bundle/nsis/*-setup.exe.sig` (when signed)<br>`target/x86_64-pc-windows-msvc/release/bundle/msi/*.msi`<br>`target/x86_64-pc-windows-msvc/release/bundle/msi/*.msi.sig` (when signed) |

---

#### 3. `release` (`needs: build`, Tagged `v*` Only)
Triggered only when the Git reference starts with `refs/tags/v`:
- **Runner**: `ubuntu-22.04`
- **Permissions**: `contents: write`
- **Artifact Download**: Downloads all build artifacts into `artifacts/` via `actions/download-artifact@v4` with `pattern: bundle-*` and `merge-multiple: false`. This places outputs in `artifacts/bundle-<target>/<bundle-type>/<file>`.
- **Staging & Manifest Generation**: Executes `bash scripts/ci/stage-release.sh`.
- **Draft GitHub Release**: Publishes a **draft** release containing all files staged in `dist-release/*` using `softprops/action-gh-release@v2`. Releases remain draft until reviewed and published manually.

---

## Secrets Reference

The workflow references the following secrets by name:

| Secret Name | Scope & Effect |
|-------------|----------------|
| `APPLE_CERTIFICATE` | Base64-encoded Apple Developer ID Application certificate (`.p12`). Required for macOS code signing. |
| `APPLE_CERTIFICATE_PASSWORD` | Passphrase to decrypt `APPLE_CERTIFICATE`. |
| `APPLE_SIGNING_IDENTITY` | Full identity string matching Keychain (e.g. `Developer ID Application: Team Name (TEAMID)`). |
| `APPLE_ID` | Apple ID email address used by notarytool for notarization. |
| `APPLE_PASSWORD` | App-specific password generated for `APPLE_ID`. |
| `APPLE_TEAM_ID` | 10-character Apple Developer Team ID. |
| `TAURI_SIGNING_PRIVATE_KEY` | Minisign private key string. When present, enables `createUpdaterArtifacts` and generates `.sig` signature files. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Passphrase protecting `TAURI_SIGNING_PRIVATE_KEY`. |
| `GITHUB_TOKEN` | Standard GitHub Actions token, used by `action-gh-release` to create draft releases. |

*Note on Windows Code Signing*: `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD` (or Azure Trusted Signing) are noted in `build.yml` comments but are not yet enabled in the step env block. Windows builds currently proceed unsigned.

If signing secrets are absent, the workflow completes an **unsigned build** without errors.

---

## Staging Script: `scripts/ci/stage-release.sh`

The staging script bridges raw build artifacts to published release assets and the Tauri v2 updater manifest:

### Execution Flow
1. **Validation & Preparation**:
   - Ensures `GITHUB_REPOSITORY` and `GITHUB_REF_NAME` are set.
   - Cleans and creates `dist-release/`.
2. **Platform Mapping**:
   - Matches directories under `artifacts/bundle-*` to platform targets:
     - `x86_64-unknown-linux-gnu` &rarr; `linux-x86_64`
     - `aarch64-unknown-linux-gnu` &rarr; `linux-aarch64`
     - `aarch64-apple-darwin` &rarr; `darwin-aarch64`
     - `x86_64-apple-darwin` &rarr; `darwin-x86_64`
     - `x86_64-pc-windows-msvc` &rarr; `windows-x86_64`
3. **Deliverable Filtering**:
   - Inspects files at exactly depth 2 (`bundle_dir/<bundle-type>/<file>`).
   - Filters by allowed deliverable extensions: `.deb`, `.rpm`, `.AppImage`, `.AppImage.sig`, `.dmg`, `.app.tar.gz`, `.app.tar.gz.sig`, `*-setup.exe`, `*-setup.exe.sig`, `.msi`, `.msi.sig`.
4. **Updater Bundle Selection**:
   - Identifies the preferred updater bundle per platform (`PLATFORM_PREFERRED`):
     - `linux-x86_64`: `AppImage`
     - `linux-aarch64`: `AppImage`
     - `darwin-aarch64`: `app.tar.gz`
     - `darwin-x86_64`: `app.tar.gz`
     - `windows-x86_64`: `setup.exe`
   - If a corresponding `.sig` signature exists for the preferred bundle, the bundle and signature are registered for the updater feed.
5. **Deterministic Renaming & Staging**:
   - Spaces in all filenames are replaced with dashes (`${basename_file// /-}`).
   - macOS updater archives and signatures are renamed to include the target architecture:
     `Hermes-Tauri_${VERSION}_${arch}.app.tar.gz` and `Hermes-Tauri_${VERSION}_${arch}.app.tar.gz.sig` (where `arch` is `aarch64` or `x86_64`), preventing cross-architecture file collisions in the flat `dist-release/` directory.
   - Verifies there are no name collisions in `dist-release/`.
   - Copies files into `dist-release/`.
6. **Updater `latest.json` Generation**:
   - If no `.sig` files exist across any platform (`sig_count == 0`), the script logs that it is an unsigned build, lists staged assets, and exits 0 without writing `latest.json`.
   - If signatures exist, generates `dist-release/latest.json` adhering to the Tauri v2 schema (`version`, `notes`, `pub_date`, and `platforms` dictionary).
   - Validates that every platform asset URL points to a staged file present in `dist-release/`.

### Local Testing

To test staging locally without a full build matrix:

```bash
# Set up mock depth-2 artifact structure
mkdir -p .oc/tmp/fake-artifacts/bundle-x86_64-unknown-linux-gnu/appimage
touch .oc/tmp/fake-artifacts/bundle-x86_64-unknown-linux-gnu/appimage/Hermes-Tauri_0.1.0_amd64.AppImage
echo "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==" > .oc/tmp/fake-artifacts/bundle-x86_64-unknown-linux-gnu/appimage/Hermes-Tauri_0.1.0_amd64.AppImage.sig

# Run staging script
GITHUB_REPOSITORY="owner/repo" GITHUB_REF_NAME="v0.1.0" \
ARTIFACTS_DIR=".oc/tmp/fake-artifacts" bash scripts/ci/stage-release.sh

# Unsigned build test (empty directory)
mkdir -p .oc/tmp/empty-artifacts
GITHUB_REPOSITORY="owner/repo" GITHUB_REF_NAME="v0.1.0" \
ARTIFACTS_DIR=".oc/tmp/empty-artifacts" bash scripts/ci/stage-release.sh
```

---

## Release Process

1. Merge approved changes into `master`.
2. Create and push a semver tag:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The `check` and `build` jobs compile all target binaries and upload artifacts.
4. The `release` job stages files, generates `latest.json` (if signing is enabled), and creates a draft release on GitHub.
5. Inspect the draft release on GitHub, verify the deliverables, and click **Publish Release**.