# Platform Support Status

This document defines the implementation, verification status, known behaviors, and outstanding requirements for each supported platform in the Hermes Tauri shell project.

---

## Status by Platform

### 1. Linux x86_64 (`x86_64-unknown-linux-gnu`)

#### Verified on the dev workstation
Verified on the Linux x86_64 dev workstation with WebKitGTK 4.1 `2.52.6`, under
`dbus-run-session -- xvfb-run -a -s "-screen 0 1440x900x24"`, against an isolated test Hermes on
ports 18800-18899:

- **Build**: a release binary and `deb`/`appimage` bundles have been produced here with
  `scripts/builder.sh` (the container has the GTK/WebKitGTK `-dev` headers); the CI `check` job runs
  `cargo fmt --check`, clippy, and the tests with `--locked`.
- **Attach mode**: `hermes-tauri --url <launch link>` connects (`Hermes backend ready at
  http://127.0.0.1:18800`), shows the tray, registers the hotkey, logs `Updates not configured`,
  and exits cleanly on `SIGTERM` (`Hermes backend stopped: Detached`).
- **Spawn mode**: `HERMES_TAURI_CMD` (a JSON argv array) starts the backend, the shell waits for
  health before navigating, and on exit the whole process group is terminated (the port is freed;
  confirmed with `ss`).
- **Native overlays**: the tray icon, the `CommandOrControl+Shift+Space` global hotkey, and the
  origin-scoped `hermesDesktop` bridge (`notify`, `openExternal`, `selectPaths`) initialise.
  Deep-link parsing is covered by the crate's unit tests; live `hermes-tauri://` activation was not
  sent in these runs.
- **Rendering**: the boot, home, team, kanban, studio, and settings routes render without console or
  page errors under WebKitGTK 2.52.6, matching Chromium ([docs/webkit-compat.md](webkit-compat.md)).
- **Measurements**: size, idle RAM, and startup figures are in
  [docs/measurements.md](measurements.md), measured with the T1 build and again with the T6 build.
- **Splash-until-paint**: the shell keeps its own boot page in a second, always-on-top window (the
  splash) until the app's page reports its first painted frame, so the window is never a flat blank
  colour while the app's JavaScript bundle executes. On Wayland the compositor decides window
  placement, so the splash and the main window may not overlap exactly as they do under X11, where
  both are centred; the splash still covers the main window because it is always on top and the
  compositor stacks it above, but a tiling compositor can place the two side by side.

Behaviour that is only exercised on other systems, or under a real desktop session, is described
below. The known cross-cutting limitations are collected in the [README](../README.md#known-limitations).

#### Written but not run (CI Matrix)
- **CI Runner**: `ubuntu-22.04` ([build.yml:96](../.github/workflows/build.yml#L96))
- **Target Triple**: `x86_64-unknown-linux-gnu` ([build.yml:97](../.github/workflows/build.yml#L97))
- **Bundles Configured**: `deb,appimage,rpm` ([build.yml:98](../.github/workflows/build.yml#L98))
- **Artifact Upload**: Uploaded as `bundle-x86_64-unknown-linux-gnu` ([build.yml:209](../.github/workflows/build.yml#L209)) containing `.deb`, `.rpm`, `.AppImage`, and conditional `.AppImage.sig` ([build.yml:213-216](../.github/workflows/build.yml#L213-216)).
- **Code Quality**: Verified in the `check` job via `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` ([build.yml:43-48](../.github/workflows/build.yml#L43-48)).

#### What is left
1. ~~Complete the Xvfb headless runtime proof~~ Done in T6: `scripts/proof.sh` drives the shell under
   `dbus-run-session -- xvfb-run` and writes 9 verified screenshots plus two checks (the reload keeps the
   session; the splash cap removes the always-on-top window). See [docs/proof.md](proof.md). WebGL/studio
   rendering under the AppImage was resolved in T22 (the blank shot was the studio's own loading state;
   see "The /studio 3D scene under Xvfb (T22)" above); the long-run memory footprint is still open.
2. Verify package installation on native host distros (Debian, Ubuntu, Fedora/RHEL, Arch) rather than containerized environments.
3. Validate desktop environment integration under both GNOME Shell (with and without the AppIndicator extension) and KDE Plasma.
4. Verify Wayland vs X11 behavior for global window toggling (X11 only via `global-hotkey`; requires active XWayland window focus on Wayland sessions) and deep-link activation.
5. Benchmark runtime memory overhead of `WebKitWebProcess` under prolonged usage.

---

### 2. Linux aarch64 (`aarch64-unknown-linux-gnu`)

#### Verified on the dev workstation
nothing verified yet

#### Written but not run (CI Matrix)
- **CI Runner**: `ubuntu-22.04-arm` ([build.yml:99](../.github/workflows/build.yml#L99))
- **Target Triple**: `aarch64-unknown-linux-gnu` ([build.yml:100](../.github/workflows/build.yml#L100))
- **Bundles Configured**: `deb,appimage` ([build.yml:101](../.github/workflows/build.yml#L101))
- **Artifact Upload**: Uploaded as `bundle-aarch64-unknown-linux-gnu` ([build.yml:209](../.github/workflows/build.yml#L209)) containing `.deb`, `.AppImage`, and conditional `.AppImage.sig` ([build.yml:213,215-216](../.github/workflows/build.yml#L213-215-216)). (RPM is not configured for ARM64).

#### What is left
1. Execute the GitHub Actions workflow on an actual `ubuntu-22.04-arm` runner to verify compilation and packaging of WebKitGTK dependencies on ARM64.
2. Test installation and launch on physical ARM64 Linux hardware (e.g. Raspberry Pi 5, Asahi Linux on Apple Silicon, Ampere Altra cloud instances).
3. Validate WebKitGTK JIT compatibility and performance on ARM64 Linux kernels.
4. Verify `WEBKIT_DISABLE_DMABUF_RENDERER=1` behavior on ARM GPU drivers (Mali, VideoCore, Panfrost) *(unconfirmed on non-Mesa drivers)*.

---

### 3. macOS ARM64 (`aarch64-apple-darwin`)

#### Verified on the dev workstation
nothing verified yet

#### Written but not run (CI Matrix)
- **CI Runner**: `macos-14` (Apple Silicon M1 runner) ([build.yml:102](../.github/workflows/build.yml#L102))
- **Target Triple**: `aarch64-apple-darwin` ([build.yml:103](../.github/workflows/build.yml#L103))
- **Bundles Configured**: `app,dmg` ([build.yml:104](../.github/workflows/build.yml#L104))
- **Artifact Upload**: Uploaded as `bundle-aarch64-apple-darwin` ([build.yml:209](../.github/workflows/build.yml#L209)) containing `.dmg`, `.app.tar.gz`, and conditional `.app.tar.gz.sig` ([build.yml:217-219](../.github/workflows/build.yml#L217-219)).
- **Staging Handling**: `scripts/ci/stage-release.sh` renames the updater archive deterministically to `Hermes-Tauri_${VERSION}_aarch64.app.tar.gz` and `.sig` to avoid collision with x64 ([stage-release.sh:115](../scripts/ci/stage-release.sh#L115)).

#### What is left
1. Provision and configure Apple Developer signing secrets in GitHub Actions: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.
2. Verify Hardened Runtime signing and Apple notarization via `notarytool` on CI.
3. Test `.dmg` installation and Gatekeeper verification on macOS 14 (Sonoma) and macOS 15 (Sequoia).
4. Verify WKWebView rendering consistency (CSS glass/backdrop filters, WebGL canvas, and session streams).
5. Verify custom URL protocol handling (`hermes-tauri://`) via `Info.plist` on macOS.

---

### 4. macOS x64 (`x86_64-apple-darwin`)

#### Verified on the dev workstation
nothing verified yet

#### Written but not run (CI Matrix)
- **CI Runner**: `macos-14` (cross-compiling on Apple Silicon runner) ([build.yml:105](../.github/workflows/build.yml#L105))
- **Target Triple**: `x86_64-apple-darwin` ([build.yml:106](../.github/workflows/build.yml#L106))
- **Bundles Configured**: `app,dmg` ([build.yml:107](../.github/workflows/build.yml#L107))
- **Artifact Upload**: Uploaded as `bundle-x86_64-apple-darwin` ([build.yml:209](../.github/workflows/build.yml#L209)) containing `.dmg`, `.app.tar.gz`, and conditional `.app.tar.gz.sig` ([build.yml:217-219](../.github/workflows/build.yml#L217-219)).
- **Staging Handling**: `scripts/ci/stage-release.sh` renames the updater archive deterministically to `Hermes-Tauri_${VERSION}_x86_64.app.tar.gz` and `.sig` to avoid collision with arm64 ([stage-release.sh:115](../scripts/ci/stage-release.sh#L115)).

#### What is left
1. Verify cross-compilation toolchain on the `macos-14` runner for Rust and C/C++ dependencies targeting Intel Macs.
2. Sign and notarize Intel x86_64 binaries through Apple notary service.
3. Validate runtime execution on physical Intel Mac hardware or Rosetta 2 translation environment.
4. Evaluate whether a universal binary (`universal2` via `lipo`) is desired or if separate architecture distributions are preferred.

---

### 5. Windows x64 (`x86_64-pc-windows-msvc`)

#### Verified on the dev workstation
nothing verified yet

#### Written but not run (CI Matrix)
- **CI Runner**: `windows-latest` ([build.yml:108](../.github/workflows/build.yml#L108))
- **Target Triple**: `x86_64-pc-windows-msvc` ([build.yml:109](../.github/workflows/build.yml#L109))
- **Bundles Configured**: `nsis,msi` ([build.yml:110](../.github/workflows/build.yml#L110))
- **Artifact Upload**: Uploaded as `bundle-x86_64-pc-windows-msvc` ([build.yml:209](../.github/workflows/build.yml#L209)) containing `*-setup.exe`, `*-setup.exe.sig` (conditional), `*.msi`, and `*.msi.sig` (conditional) ([build.yml:220-223](../.github/workflows/build.yml#L220-223)).
- **Code Signing**: Currently commented out in [build.yml:155-157](../.github/workflows/build.yml#L155-157). Builds run unsigned by default.
- **Code Quality**: The Linux `check` job installs the Windows target and runs `cargo check --all-targets` and `cargo clippy --all-targets -D warnings` for `hermes-sidecar` and `hermes-shell-config` ([build.yml:57-86](../.github/workflows/build.yml#L57-86)), so Windows-only code fails CI instead of waiting for this runner. No lint is allowed there: the one that used to be (`clippy::result_large_err` for `ConfigError::ConfigFile`) is fixed by boxing that error's `toml::de::Error`.

#### Fixed since the R7 audit (compile-checked, never run)
The R7 audit found that no `cfg(windows)` code had ever been compiled. These are fixed; all of them are type-checked for `x86_64-pc-windows-msvc` and none has been executed:
- `crates/hermes-sidecar/Cargo.toml` enables the two `windows-sys` features the Job Object code needs: `Win32_Security` for `CreateJobObjectW` (its signature takes `SECURITY_ATTRIBUTES`) and `Win32_System_Threading` for `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` (it embeds `IO_COUNTERS`).
- `ProcessGroup::has_live_members` exists on Windows ([process/windows.rs](../crates/hermes-sidecar/src/process/windows.rs)), so the shutdown wait in `backend.rs` compiles there.
- A `.cmd`/`.bat` backend is now run through `cmd.exe` with a command line the shell builds itself ([src/batch.rs](../crates/hermes-sidecar/src/batch.rs), unit-tested on Linux). `CreateProcessW` cannot start a batch file, and Rust's `Command` only manages it through an undocumented `CreateProcessW` feature that the standard library documents as something that "may be removed in the future" (`std::process`, "Batch file special handling"). This is the path the `hermes.cmd` entry in `EXECUTABLE_NAMES` takes.
- The "Prepare signing environment" step of the build workflow sets `shell: bash`; without it the step failed on `windows-latest`, where `run:` defaults to PowerShell, before the build started.

#### Still open (R7)
1. **Job Object assignment happens after spawn**: the child is assigned once `spawn` has returned, so a grandchild started in that window escapes the job. Assigning a child created suspended (or with `PROC_THREAD_ATTRIBUTE_JOB_LIST`) would close it.
2. **No graceful stop**: `request_stop` returns `false` on Windows, so every shutdown is a hard `TerminateJobObject`; the plan's `taskkill /T` fallback is not implemented. `AssignProcessToJobObject` can also fail with `ERROR_ACCESS_DENIED` when the shell already runs inside a job (CI, some launchers), and `adopt` then kills the child and reports a failed start.
3. **No Windows payload launcher**: `bundled_launcher` returns `None` off Unix and the Unix launcher path has no `.exe`, so `Source::Bundled` is unreachable on Windows. A Windows payload needs `hermes-payload\bin\hermes-sidecar.exe` (or `.cmd`), `python.exe` from python-build-standalone as the venv entry point, the payload registered as a Tauri resource for `x86_64-pc-windows-msvc`, and `is_executable_file` checking `.exe`/`.cmd`/`.bat` instead of accepting any regular file.

#### What is left
1. Run the CI build workflow on `windows-latest` to confirm NSIS and WiX (MSI) bundle generation.
2. Configure production code signing via Azure Trusted Signing or a PFX certificate to satisfy Microsoft SmartScreen.
3. Test runtime execution and process lifecycle termination on Windows 10 and Windows 11.
4. Validate that the compile-gated Windows Job Object implementation in [crates/hermes-sidecar/src/process/windows.rs](../crates/hermes-sidecar/src/process/windows.rs) reliably terminates child and grandchild processes upon application exit (see "Still open" above).
5. Verify deep link registry entries written by the NSIS and MSI installers and confirm single-instance forwarding.
6. Prove the `.cmd` backend path end to end: `cmd.exe` is started with the command line `src/batch.rs` builds, and `hermes.cmd` resolves through `PATH`.

---

## Technical Architecture & Upstream Behaviors

### 1. WebKitGTK Version Floor & DMABUF Renderer Workaround

#### Version Floor
The Linux shell dynamically links against WebKitGTK 4.1 (`libwebkit2gtk-4.1.so.0`). Compatibility findings from the WebKit audit ([webkit-compat.md](webkit-compat.md)):
- **Dev Workstation Runtime**: WebKitGTK 4.1 `2.52.6-0ubuntu0.26.04.1` (Safari 26.4–26.6 era).
- **Builder Container Image**: `hermes-tauri-builder:26.04` ships WebKitGTK 4.1 `2.52.6-0ubuntu0.26.04.1`.
- **CI APT Package Floor**: `ubuntu-22.04` ships WebKitGTK `2.50.4-0ubuntu0.22.04.1` (verified at [Launchpad Ubuntu Jammy webkit2gtk](https://launchpad.net/ubuntu/jammy/+source/webkit2gtk)).
- **Feature Baseline**: Hermes Desktop relies on CSS and ECMAScript features establishing an engine floor around Safari 15.4 / WebKitGTK 2.36:
  - Required CSS: `:has()` (Safari 15.4), `@layer` (Safari 15.4), `color-mix()` (Safari 16.2), relative `oklch()` (Safari 16.4).
  - Required JS APIs: `structuredClone()` (Safari 15.4), `Array.prototype.findLast()` (Safari 15.4), `BroadcastChannel` (Safari 15.4).
  - Missing JS API: `requestIdleCallback` is absent in WebKitGTK 2.52.6 and degrades to a `setTimeout` fallback shim.

#### DMABUF Renderer Workaround
WebKitGTK 2.42+ introduced hardware-accelerated buffer sharing via DMA-BUF by default. On Linux systems using NVIDIA proprietary drivers, Wayland compositors, or headless Xvfb environments, this frequently causes blank/white rendering, EGL errors (`EGL_BAD_PARAMETER`), or protocol crashes:
- WebKit Bug Tracker: [WebKit Bug 261874](https://bugs.webkit.org/show_bug.cgi?id=261874)
- WebKit Bug Tracker: [WebKit Bug 262607](https://bugs.webkit.org/show_bug.cgi?id=262607)
- Upstream Tauri Issue: [tauri-apps/tauri #8421](https://github.com/tauri-apps/tauri/issues/8421)

**Mitigation**: The shell explicitly sets `std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1")` before initializing the webview unless overridden by the user.

#### The /studio 3D scene under Xvfb (T22)

T16 recorded the studio page's 3D scene as blank under the payload AppImage (19 distinct colours on
the proof's 32x20 sample grid vs 215 with the system WebKitGTK, same script) and left the root cause
open. T22 reproduced a blank-looking studio shot but showed it is the page's own "building the
studio…" loading state, not a rendering-engine failure:

- **Versions are identical.** The payload AppImage bundles WebKitGTK `2.52.6-0ubuntu0.26.04.1`
  (package stamp inside the AppDir), the same release as the system's `libwebkit2gtk-4.1-0`
  (`dpkg -l`). The AppImage bundles no Mesa/GL libraries, so both sides render GL through the same
  host stack.
- **The bundled-WebKit side renders the scene.** The T16 payload AppImage with its default
  environment (the shell's `WEBKIT_DISABLE_DMABUF_RENDERER=1` included) built and rendered the full
  3D scene under Xvfb in 3 of 3 runs (195, 205, 199 grid colours), 16-26 s after the deep link.
- **The blank state is the loading state.** The scene build (three.js plus WASM workers under
  software GL) passes through low-colour frames for 10-25 s: the repeat run scored 19 then 12 before
  rendering 205 at t=23 s. T16's 19-colour shot caught exactly this state, and `MIN_COLOURS=12`
  could not reject it.
- **Processes and sandbox are not factors.** `WebKitWebProcess` and `WebKitNetworkProcess` stayed
  alive in every AppImage run while the scene was pending, and
  `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1` rendered too (199). `GSK_RENDERER=cairo`, "stuck" in
  an earlier run, rendered under a healthy machine (202).
- **An earlier knob matrix is void.** The first T22 matrix was run under machine-wide memory
  pressure (the runner was OOM-killed mid-run); with a healthy machine every cell renders, including
  cells whose environment was identical to the default's.
- **Proof change.** `scripts/proof.sh` gives the `/studio` shot its own bar, `STUDIO_MIN_COLOURS=100`:
  only the built scene reaches it, so a studio stuck on its loading state fails the proof. See
  [proof.md](proof.md).

---

### 2. AppImage WebKitGTK Bundling vs. Distribution WebKitGTK

The distribution packaging model differs fundamentally between the AppImage container and native package manager formats (`.deb` / `.rpm`):

#### Built AppDir Inspection & Bundled Payload
Unlike native system packages, the Tauri AppImage **bundles WebKitGTK and its runtime dependencies directly from the build machine**. Inspection of the built AppDir directory confirms the bundled libraries:
- `usr/lib/libwebkit2gtk-4.1.so.0`
- `usr/lib/libjavascriptcoregtk-4.1.so.0`
- `usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitWebProcess`
- `usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/injected-bundle/libwebkit2gtkinjectedbundle.so`

Because WebKitGTK, JavaScriptCore, and their runtime helper processes are bundled, the AppDir's `usr/lib` directory is ~252 MB, producing an AppImage bundle of ~85.7 MiB (in contrast to the native `.deb` package, which is only ~6.84 MiB). The T6 build measures 87.39 MiB AppImage and 8.54 MiB deb from the same AppDir layout.

#### Glibc Baseline & Oldest Supported Distro
The official Tauri AppImage documentation states:
> "AppImage is a distribution format that does not rely on the system installed packages and instead bundles all dependencies and files needed by the application. For this reason, the output file is larger but easier to distribute since it is supported on many Linux distributions and can be executed without installation."
>
> "Still, you should carefully use it as the file size grows from the 2-6 MB range to 70+ MB."
>
> "Core libraries such as glibc frequently break compatibility with older systems. For this reason, you must build your Tauri application using the oldest base system you intend to support that also provides Tauri v2’s required WebKitGTK 4.1 packages. Ubuntu 22.04 and Debian 12 are suitable baseline examples because they provide `libwebkit2gtk-4.1-dev` from their standard package repositories. Building on a newer base system can raise the minimum glibc version required by your app, so when running on an older system, you may face a runtime error like `/usr/lib/libc.so.6: version 'GLIBC_2.33' not found`."

Because the bundled WebKitGTK binaries are compiled against the build machine's C runtime library (`glibc`), building on a newer distribution ties the AppImage to that distribution's `glibc` symbols. CI builds Linux artifacts on `ubuntu-22.04` to ensure maximum backwards compatibility with target host distributions.

#### Deb & RPM Contrast
In contrast, native Debian (`.deb`) and Red Hat (`.rpm`) packages do not bundle WebKitGTK. Instead, they declare package manager dependencies:
- Debian/Ubuntu: `Depends: libwebkit2gtk-4.1-0, libgtk-3-0`
- Fedora/RHEL: `Requires: webkit2gtk4.1, gtk3`

When installed via package managers, the application dynamically links to the host system's native WebKitGTK shared library.

#### Host Driver Interaction
Even though the AppImage packages its own WebKitGTK binary tree, the bundled WebKitGTK process still interfaces at runtime with the host display server (X11 or Wayland), D-Bus session bus, font config, and host GPU hardware drivers (via Mesa / DRM / EGL / proprietary vendor drivers). Consequently, graphics driver behaviors and acceleration quirks (such as DMA-BUF incompatibilities on NVIDIA systems) still depend on the host workstation configuration.

- Official Tauri AppImage Documentation: [Tauri AppImage Distribution](https://v2.tauri.app/distribute/appimage/)

---

### 3. Linux Package Self-Update Mechanisms & Limitations

Tauri provides application self-updating through the `tauri-plugin-updater` crate:

#### Upstream Support in `tauri-plugin-updater` (Checked: v2.14.0)
Historically, Linux auto-updating in desktop frameworks supported AppImage only. However, upstream pull request [tauri-apps/plugins-workspace #2624](https://github.com/tauri-apps/plugins-workspace/pull/2624) introduced bundle-specific updater targets, adding support for **`.deb`** and **`.rpm`** packages alongside **`.AppImage`** in `tauri-plugin-updater` v2 (verified in `tauri-plugin-updater` v2.14.0, [plugins/updater/src/updater.rs](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/updater)).

In `updater.rs` (lines 1213–1430), the update engine inspects the current installation type via `installer_for_bundle_type(bundle_type())`:
1. **AppImage (`install_appimage`)**:
   - Operates entirely in user-space without root elevation.
   - Replaces the current running AppImage executable in-place (or unpacks a `.tar.gz` compressed AppImage when the `zip` feature is active), creating a backup in a temporary directory on the same mount point, writing the payload, and setting executable permissions (`0o700`).
2. **Debian (`install_deb`)**:
   - Validates the downloaded payload (`infer::archive::is_deb`).
   - Writes the `.deb` package to a temporary path (`tauri_deb_update/package.deb`).
   - Invokes `try_install_with_privileges(&pkg_path, "dpkg", "-i")`.
3. **RPM (`install_rpm`)**:
   - Validates the downloaded payload (`infer::archive::is_rpm`).
   - Writes the `.rpm` package to a temporary path (`tauri_rpm_update/package.rpm`).
   - Invokes `try_install_with_privileges(&pkg_path, "rpm", "-U")`.

#### Privilege Elevation Mechanism for Deb / RPM
Because `.deb` and `.rpm` packages install files into root-owned directories (`/usr/bin/`, `/usr/share/`), `try_install_with_privileges` attempts privilege escalation through three tiered strategies:
1. `pkexec <cmd> <arg> <pkg_path>`: Attempts PolicyKit graphical authentication.
2. GUI Sudo: Attempts graphical password acquisition via `zenity --password` or `kdialog --password` and pipes the credentials to `sudo -S`.
3. Terminal Sudo: Executes `sudo <cmd> <arg> <pkg_path>` directly.

#### Hermes Tauri Release Configuration
While upstream `tauri-plugin-updater` provides `.deb` and `.rpm` installation logic, the Hermes Tauri release staging script ([stage-release.sh:38-44](../scripts/ci/stage-release.sh#L38-L44)) defines `PLATFORM_PREFERRED`:
- `linux-x86_64`: `AppImage`
- `linux-aarch64`: `AppImage`

Consequently, our automated release manifest (`latest.json`) publishes AppImage bundles as the primary updater target for Linux. Debian and RPM packages staged in `dist-release/` are delivered as standalone package installer artifacts.

- Official Tauri Updater Documentation: [Tauri Updater Plugin](https://v2.tauri.app/plugin/updater/)
- Upstream PR: [tauri-apps/plugins-workspace #2624](https://github.com/tauri-apps/plugins-workspace/pull/2624)

---

### 4. Linux System Tray & StatusNotifier Host

Tauri v2 exposes system tray functionality through the `tray-icon` crate and `libayatana-appindicator3` / `libappindicator3`:
- On Linux, tray icons are served over D-Bus adhering to the **StatusNotifierItem** (SNI) protocol.
- **Desktop Environments**:
  - **KDE Plasma & XFCE**: Provide native StatusNotifierItem hosts; tray icons display out of the box.
  - **GNOME Shell**: Dropped legacy tray support and does not provide an SNI host natively. Tray icons will not appear on default GNOME sessions unless the user installs the [AppIndicator and KStatusNotifierItem Support extension](https://extensions.gnome.org/extension/615/appindicator-support/).
- Official Tauri Tray Guide: [Tauri System Tray](https://v2.tauri.app/learn/system-tray/)

---

### 5. Global Hotkeys under Wayland vs X11

The `tauri-plugin-global-shortcut` manages system-wide keyboard shortcuts:

#### Upstream Engine: `global-hotkey`
The plugin is built directly on top of the `global-hotkey` crate ([tauri-apps/global-hotkey](https://github.com/tauri-apps/global-hotkey)).
- **Platforms Supported**: The upstream `global-hotkey` README explicitly specifies:
  ```markdown
  ## Platforms-supported:
  - Windows
  - macOS
  - Linux (X11 Only)
  ```
- **X11 Implementation**: On Linux, the backend in `src/platform_impl/x11/mod.rs` connects to the X server using `x11rb::rust_connection::RustConnection` and registers hotkeys via `xproto::grab_key` (`XGrabKey`) on the root window, iterating across `ignored_mods()` (NumLock and CapsLock combinations).

#### Wayland Limitations & XWayland Behavior
- **No Portal Implementation**: Neither `global-hotkey` nor `tauri-plugin-global-shortcut` implements or consumes the XDG Desktop Portal GlobalShortcuts interface (`org.freedesktop.portal.GlobalShortcuts`).
- **Wayland Security Model**: Native Wayland compositors isolate input between client surfaces to protect against keylogging. Traditional X11 key grabbing (`XGrabKey`) cannot listen to keystrokes on native Wayland surfaces.
- **Behavior Under Wayland Sessions**: On a Wayland desktop, hotkeys registered through `tauri-plugin-global-shortcut` function **only via XWayland while an X11/XWayland window has active keyboard focus**. If the active window is a native Wayland surface or if the application is unfocused or backgrounded, hotkey events are not intercepted.

#### Desktop Portal Support Status
While modern desktop environments have introduced portal-mediated global shortcuts:
- KDE Plasma added `org.freedesktop.portal.GlobalShortcuts` support in Plasma 5.27+.
- GNOME did **not** ship GlobalShortcuts portal support in GNOME 44 (which only added input capture and session groundwork); portal support in `xdg-desktop-portal-gnome` was introduced via Merge Request !208 and debuted in **GNOME 48**.
- wlroots-based compositors (Sway, Hyprland) do not implement the GlobalShortcuts portal.

Because `global-hotkey` lacks a portal backend, global shortcuts currently remain functional exclusively under native X11 or focused XWayland windows.

- Official Tauri Plugin Documentation: [Tauri Global Shortcut Plugin](https://v2.tauri.app/plugin/global-shortcut/)
- Upstream Crate Repository: [tauri-apps/global-hotkey](https://github.com/tauri-apps/global-hotkey)
- XDG Specification: [XDG Desktop Portal GlobalShortcuts](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html)
- GNOME Portal Implementation: [xdg-desktop-portal-gnome MR !208](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/merge_requests/208)

---

### 6. Deep Links & Single-Instance Forwarding

The application registers the custom URL protocol `hermes-tauri://`:
- **Linux**: Registered in the desktop file via `MimeType=x-scheme-handler/hermes-tauri;` and `Exec=hermes-tauri %u`.
- **macOS**: Registered in `Info.plist` under `CFBundleURLTypes` specifying `CFBundleURLSchemes` with `hermes-tauri`.
- **Windows**: Registered in the Windows Registry under `HKEY_CURRENT_USER\Software\Classes\hermes-tauri` with a shell open command pointing to the executable. NSIS and WiX installers write these registry entries during installation.
- **Single-Instance Forwarding**: When a user clicks a `hermes-tauri://` link while the application is already running:
  - On macOS, deep link events are delivered via Apple Events to the running process.
  - On Windows and Linux, the operating system executes a new secondary process with the URL as a command-line argument. The `tauri-plugin-single-instance` plugin (compiled with feature `deep-link`) detects the primary instance, delivers the arguments across the IPC channel, brings the primary window to the front, and exits the secondary process immediately.
- Official Tauri Documentation: [Tauri Deep Link Plugin](https://v2.tauri.app/plugin/deep-link/) and [Tauri Single Instance Plugin](https://v2.tauri.app/plugin/single-instance/)

---

### 7. GTK Application ID & D-Bus Session Bus Coordination

On Linux, application uniqueness and inter-process handoff rely on the user's D-Bus session bus:
- The `tauri-plugin-single-instance` registers a well-known D-Bus name (derived from the app identifier, e.g. `org.<identifier>.SingleInstance`) on the session bus.
- If an instance is already holding the name, the secondary instance sends a D-Bus method call carrying its command-line arguments to the primary instance and terminates cleanly.
- In containerized or flatpak sandboxes, session bus talk/own permissions for this service name must be explicitly granted.
- GIO Reference: [GApplication D-Bus Documentation](https://docs.gtk.org/gio/class.Application.html)

---

### 8. macOS Code Signing, Hardened Runtime, and Notarization

Distributing macOS desktop applications outside the Mac App Store requires strict compliance with Apple Gatekeeper:
- **Developer ID**: Binaries must be signed using a valid Apple Developer ID Application certificate.
- **Hardened Runtime**: Enabled during signing (`--options runtime`). Allows macOS to enforce security mitigations while honoring declared entitlements.
- **Entitlements**: Specified in `Entitlements.plist` (e.g. `com.apple.security.cs.allow-jit`, `com.apple.security.network.client`).
- **Notarization**: Signed archives (`.app.tar.gz` and `.dmg`) are uploaded to Apple's notary service using `notarytool`. Once approved, the notarization ticket is stapled to the application bundle using `stapler`.
- Official Tauri Documentation: [Tauri macOS Code Signing](https://v2.tauri.app/distribute/sign/macos/)
- Apple Documentation: [Apple Hardened Runtime](https://developer.apple.com/documentation/security/hardened_runtime) and [Apple Notarizing Software](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution)

---

### 9. Windows WebView2 Bootstrapper Modes & Code Signing

#### WebView2 Installation Modes
Tauri requires Microsoft Edge WebView2 on Windows. The installation strategy is configured in `tauri.conf.json` under `bundle > windows > webviewInstallMode`:
1. `downloadBootstrapper` (Default): The installer checks for WebView2 and downloads the official Microsoft bootstrapper online if missing. Adds minimal installer size overhead.
2. `embedBootstrapper`: Embeds Microsoft's bootstrapper binary (~1.8 MB overhead) inside the installer executable.
3. `offlineInstaller`: Embeds the complete standalone WebView2 runtime (~127 MB overhead). Guarantees installation in strictly offline or air-gapped environments, and is required for Microsoft Store submission.
4. `skip`: Skips checking or installing WebView2 entirely. The app will fail to launch if the user lacks WebView2.
- Configuration Reference: [Tauri Windows WebviewInstallMode](https://v2.tauri.app/reference/config/#webviewinstallmode)

#### Code Signing
Unsigned Windows executables trigger Microsoft Defender SmartScreen warnings ("Windows protected your PC"). Production builds require code signing:
- Traditional signing with a standard or EV code signing certificate via `signtool.exe`.
- Cloud-based signing via Microsoft Azure Trusted Signing (formerly Azure Code Signing).
- Official Tauri Documentation: [Tauri Windows Code Signing](https://v2.tauri.app/distribute/sign/windows/)
- Microsoft Documentation: [Azure Trusted Signing](https://learn.microsoft.com/en-us/azure/trusted-signing/)

---

### 10. Sidecar Process Tree Management via Windows Job Objects

On Windows, child processes do not share Unix-style process group semantics (`setpgid` / `kill(-pid, SIGKILL)`). If a parent process crashes or terminates abruptly, child and grandchild processes become orphaned background tasks.
- **Implementation**: In [crates/hermes-sidecar/src/process/windows.rs](../crates/hermes-sidecar/src/process/windows.rs), process management uses the Windows Job Object API:
  - An unnamed job object is created via `CreateJobObjectW`.
  - Configured with `SetInformationJobObject` setting `JobObjectExtendedLimitInformation` with `LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
  - The spawned sidecar process is assigned to the job object via `AssignProcessToJobObject`.
  - When the Tauri application exits and closes the job handle (`CloseHandle`), the Windows kernel automatically terminates all processes in the job object.
- **Status & Caveats**: Compile-gated via `#[cfg(windows)]`. Fully implemented in the crate, but never run on the Linux dev workstation. Note that assignment occurs immediately after spawn, meaning any grandchild process spawned in that brief initial window could escape the job.
- Microsoft Documentation: [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)

---

### 11. Python Sidecar Packaging: `externalBin` vs Resources

Bundling the Hermes Python backend and the `hermes_plus` plugin presents two primary architectural strategies:

| Strategy | Mechanism | Startup Latency | Distribution Size Overhead | macOS Signing Requirements |
|----------|-----------|-----------------|----------------------------|----------------------------|
| **PyInstaller (`externalBin`)** | Frozen single-file executable named `<binary>-<target-triple>` in `src-tauri/binaries/` | Higher (unpacks archive to temporary filesystem on cold start) | ~30–50 MB per target executable | Single executable signed; one-folder requires signing every unpacked `.dylib` |
| **Relocatable Python (`resources`)** | Self-contained Python tree (e.g. `python-build-standalone` or `uv python install`) in bundle resources | Lower (direct execution of `bin/python` without decompression) | ~40–70 MB per platform directory | Every Mach-O binary and dynamic library (`.dylib`, `.so`) in Resources must be signed |

#### macOS Signing Constraints for Bundled Python
Apple notarization enforces that **every binary executable, shared library (`.dylib`), and C extension (`.so`)** within the application bundle must be signed with a valid Developer ID and Hardened Runtime enabled. When bundling a full Python environment or PyInstaller directory, all dynamic libraries in `site-packages/` must be recursively signed before packaging into `.dmg` or `.app.tar.gz`.

- Official Tauri Sidecar Documentation: [Tauri Sidecars](https://v2.tauri.app/develop/sidecar/)
- Official Tauri Resources Documentation: [Tauri Resources](https://v2.tauri.app/reference/config/#resources)
- Standalone Python Distribution: [python-build-standalone](https://github.com/indygreg/python-build-standalone)

---

### 12. Updater Feed Hosting & Key Management

The Tauri v2 updater enforces cryptographic verification using **Minisign**:
- **Key Generation**:
  - Keys are generated with `cargo tauri signer generate -w ~/.tauri/myapp.key`.
  - The public key is embedded in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
  - The private key is stored securely in CI secrets as `TAURI_SIGNING_PRIVATE_KEY` with optional password `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- **Manifest Format**: The feed URL serves a JSON document (`latest.json`) matching the Tauri v2 schema:
  ```json
  {
    "version": "v0.1.0",
    "notes": "Release description",
    "pub_date": "2026-10-15T00:00:00Z",
    "platforms": {
      "linux-x86_64": {
        "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
        "url": "https://github.com/owner/repo/releases/download/v0.1.0/Hermes-Tauri_0.1.0_amd64.AppImage"
      },
      "darwin-aarch64": {
        "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
        "url": "https://github.com/owner/repo/releases/download/v0.1.0/Hermes-Tauri_0.1.0_aarch64.app.tar.gz"
      },
      "darwin-x86_64": {
        "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
        "url": "https://github.com/owner/repo/releases/download/v0.1.0/Hermes-Tauri_0.1.0_x86_64.app.tar.gz"
      },
      "windows-x86_64": {
        "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
        "url": "https://github.com/owner/repo/releases/download/v0.1.0/Hermes-Tauri_0.1.0_x64-setup.exe"
      }
    }
  }
  ```
- **Hosting**: Hosted over HTTPS on a static file provider (GitHub Releases, Cloudflare R2, AWS S3, or custom CDN).
- **Key Security & Rotation**: The private signing key must never be committed to source control or exposed in client bundles. If a key is compromised, rotation requires issuing an update signed with the old key that configures the new public key in `tauri.conf.json`, or requiring manual user reinstallation.
- Official Tauri Updater Documentation: [Tauri Updater Plugin](https://v2.tauri.app/plugin/updater/)
- Minisign Specification: [Minisign Project](https://jedisct1.github.io/minisign/)
