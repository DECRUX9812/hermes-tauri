# CI/CD Documentation

## Overview

This document describes the GitHub Actions workflow for building the Hermes Tauri application across multiple platforms.

## Workflow: `.github/workflows/build.yml`

### Jobs

#### 1. `check` (ubuntu-22.04)
Runs code quality checks on every push, PR, and tag:
- **Formatting**: `cargo fmt --all -- --check`
- **Linting**: `cargo clippy --workspace --all-targets -- -D warnings`
- **Tests**: `cargo test --workspace`

Installs Tauri Linux dependencies: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, `libssl-dev`, `patchelf`.

Uses `dtolnay/rust-toolchain@stable` with `rustfmt` and `clippy` components, and `Swatinem/rust-cache@v2` for dependency caching.

#### 2. `build` (matrix, needs: check)
Builds the Tauri application for multiple targets. Uses `tauri-apps/tauri-action@v0` (chosen over manual `cargo install tauri-cli` because it handles cross-compilation setup, tauri-cli installation, and signing integration in a single maintained action).

| Platform | Runner | Target | Bundles |
|----------|--------|--------|---------|
| Linux x64 | ubuntu-22.04 | x86_64-unknown-linux-gnu | deb, appimage, rpm |
| Linux ARM64 | ubuntu-22.04-arm | aarch64-unknown-linux-gnu | deb, appimage |
| macOS ARM64 | macos-14 | aarch64-apple-darwin | app, dmg |
| macOS x64 (cross) | macos-14 | x86_64-apple-darwin | app, dmg |
| Windows x64 | windows-latest | x86_64-pc-windows-msvc | nsis, msi |

**Fail-fast: false** — one platform failure doesn't cancel others.

Artifacts uploaded: `target/<target>/release/bundle/**` (one artifact per matrix entry).

#### 3. `release` (needs: build, only on `v*` tags)
Creates a **draft** GitHub release (never auto-published):
- Downloads all build artifacts
- Generates `latest.json` for the Tauri v2 updater **only when `.sig` files exist** (i.e., when signing secrets are configured)
- Uploads all bundles and signatures as release assets
- Uses `contents: write` permission scoped to this job only (top-level is `contents: read`)

## Secrets

| Secret | Purpose | Required |
|--------|---------|----------|
| `APPLE_CERTIFICATE` | macOS Developer ID Application certificate (base64) | No |
| `APPLE_CERTIFICATE_PASSWORD` | Certificate password | No |
| `APPLE_SIGNING_IDENTITY` | Signing identity name (e.g., "Developer ID Application: Name (TEAMID)") | No |
| `APPLE_ID` | Apple ID for notarization | No |
| `APPLE_PASSWORD` | App-specific password for notarization | No |
| `APPLE_TEAM_ID` | Apple Team ID | No |
| `WINDOWS_CERTIFICATE` | Windows code signing certificate (PFX, base64) — or use Azure Trusted Signing | No |
| `WINDOWS_CERTIFICATE_PASSWORD` | Certificate password | No |
| `TAURI_SIGNING_PRIVATE_KEY` | Minisign private key for updater signatures | No |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Minisign key password | No |

**Important**: The workflow does not pass empty signing env vars to `tauri-action`. If secrets are unset, the build succeeds **unsigned**. Setting an empty secret would trigger a signing attempt and fail.

### Windows Signing Note
For Windows, you can use either:
- Traditional code signing certificate (`WINDOWS_CERTIFICATE` + `WINDOWS_CERTIFICATE_PASSWORD`)
- **Azure Trusted Signing** (recommended): Configure via `AZURE_TRUSTED_SIGNING_*` secrets (not in this workflow; add when ready)

## Per-Platform Notes

### Linux (ubuntu-22.04)
- **WebKitGTK version floor**: Ubuntu 22.04 ships WebKitGTK 2.41+. The app sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at runtime to avoid DMABUF renderer issues on NVIDIA and in headless/Xvfb environments.
- **AppImage limitation**: AppImage cannot bundle WebKitGTK cleanly (it links the host's). The AppImage will only run on distros with compatible WebKitGTK.
- **Deb/RPM**: These packages cannot self-update via Tauri updater (same as Electron). Users must use their package manager.

### macOS (macos-14)
- **Notarization**: Requires all Apple secrets (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, certificate). Without them, the `.app` and `.dmg` are built but not notarized.
- **Universal binaries**: Not configured. The matrix builds arm64 and x64 separately. Use `lipo` to combine if needed.
- **Hardened Runtime**: Enabled by Tauri v2. Entitlements are in `src-tauri/Entitlements.plist`.

### Windows (windows-latest)
- **WebView2**: The `windows-latest` runner includes the WebView2 bootstrapper. The NSIS/MSI installers will install WebView2 if missing on the target machine.
- **Signing**: Requires `WINDOWS_CERTIFICATE` (PFX) or Azure Trusted Signing. Without signing, Windows SmartScreen will warn users.

## How to Cut a Release

1. Ensure all desired changes are on `master`
2. Create and push a version tag:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The `build` job runs for all platforms; the `release` job creates a **draft** release
4. Go to GitHub Releases, review the draft, verify assets, then **Publish release**
5. The Tauri updater will pick up the published release (if `latest.json` was generated with signatures)

## Local Development

```bash
# Install Tauri CLI
cargo install tauri-cli --version '^2' --locked

# Build for current platform
cargo tauri build

# Build for specific target (requires cross-compilation setup)
cargo tauri build --target x86_64-unknown-linux-gnu --bundles deb,appimage,rpm
```

## Troubleshooting

- **Linux build fails with WebKitGTK errors**: Ensure all `-dev` packages are installed. The workflow installs them automatically.
- **macOS codesign fails**: Verify certificate is a "Developer ID Application" cert, not "Mac App Store". Check `APPLE_SIGNING_IDENTITY` matches the cert's common name.
- **Windows build fails**: Ensure `patchelf` equivalent isn't needed; WebView2 bootstrapper is on the runner.
- **Updater `latest.json` missing**: This is expected if no signing keys are configured. The updater only works with signed releases.