# Desktop shell measurements

Size, idle RAM and window startup for the Electron shell and the Tauri shell, measured the same way on
one machine. The harness is `scripts/measure.sh` (bash entry point) with `scripts/measure-helper.py`
(stdlib only, Python 3.14). Every measurement prints one JSON line to stdout and a summary to stderr.

## Results

The Tauri column is the **T6 build** (T5 integration plus the splash-until-paint fix), measured in this
task with the T12 wrapper. The T1 column is kept as history: it is the scaffold build without the tray,
hotkey, deep links or the splash.

| Metric | Electron (v0.17.0 build) | Tauri (T1 build, attach mode; tray/hotkey/updater not yet included) | Tauri (T6 build, attach mode, full T5 shell + splash) | Ratio (Tauri T6 / Electron) |
|---|---|---|---|---|
| Size, `linux-unpacked` / raw binary (apparent) | 335.5 MB (351,822,445 bytes, 434 files) | 19.4 MB (20,358,904 bytes, 1 file) | 23.85 MiB (25,008,328 bytes, 1 file) | 0.071x (-92.9%) |
| Size, `linux-unpacked` / raw binary (on disk) | 336.5 MB | 19.4 MB (20,361,216 bytes) | 23.86 MiB (25,014,272 bytes) | 0.071x (-92.9%) |
| Size, AppImage | 142 MB, from an earlier build, not re-measured | 85.72 MiB (89,881,080 bytes) * | 87.39 MiB (91,634,168 bytes) | 0.615x (-38.5%) |
| Size, deb | 114 MB, from an earlier build, not re-measured | 6.84 MiB (7,173,418 bytes) * | 8.54 MiB (8,958,852 bytes) | 0.075x (-92.5%) |
| Idle RAM, shell PSS median (run 1 / run 2 / run 3) | 414.3 MiB / 436.8 MiB | 390.2 MiB (runs: 390.2 / 390.7 / 388.6) | 403.0 MiB (runs: 405.8 / 400.2) † | 0.973x (-2.7%) |
| Idle RAM, shell RSS median (run 1 / run 2 / run 3) | 876.2 MiB / 878.3 MiB | 571.5 MiB (runs: 571.5 / 571.8 / 570.0) | 711.5 MiB (runs: 713.7 / 709.2) † | 0.812x (-18.8%) |
| Idle RAM, processes in shell tree | 7 | 3 (`hermes-tauri`, `WebKitWebProcess`, `WebKitNetworkProcess`) | 17 (3 shell processes plus the session bus and portal helpers `dbus-run-session` activates) † | 0.43x for the shell itself |
| RAM after sidebar click, PSS median | 395.5 MiB (RSS 836.3 MiB; Messaging page) | 391.8 MiB (RSS 573.1 MiB; runs: 391.8 / 392.9 / 388.1; `#/team`) | 415.3 MiB (RSS 702.6 MiB; `#/team`) † | 1.050x PSS / 0.840x RSS |
| Window startup, median of 3 cold runs | 2434.1 ms (runs: 2211.8, 2514.3, 2434.1) | 6725.4 ms (runs: 10262.2, 6725.4, 6481.1) | 2024.7 ms (runs: 2024.7, 2839.1, 1834.8) — the splash | 0.832x |
| Window startup to the remote UI painted, median of 3 | 2434.1 ms (same event for Electron: no local splash) | not measured (no splash existed) | 6443.0 ms (runs: 7328.3, 6234.3, 6443.0) | 2.65x |
| **Ratio (Tauri T6 / Electron)** | **1.0x baseline** | — | **Binary 0.071x, Deb 0.075x, AppImage 0.615x, Idle RSS 0.812x, Idle PSS 0.973x, Startup-to-splash 0.832x, Startup-to-remote-UI 2.65x** | **—** |

\* Bundle files under `$HOME/.cache/hermes-tauri/target/release/bundle/` had mtimes from 01:33–01:34 UTC (concurrent T5 build), so frozen T1 bundle sizes from `.oc/brief/T1-report.md` are reported. Raw binary measured directly from `$HOME/.cache/hermes-tauri/bin-t1/hermes-tauri`.

† The T6 RAM runs use the T12 wrapper form (`dbus-run-session -- <binary>`, a private session bus per run),
which the Electron and T1 runs did not. The harness counts every process in the measured session, so the
T6 tree also holds the session bus, the accessibility bus and the XDG portal helpers that bus activates
(11–14 extra processes, about 20 MiB PSS together). The shell-only subset — `hermes-tauri`,
`WebKitWebProcess`, `WebKitNetworkProcess` — is 384.8 / 379.2 MiB PSS idle (median 382.0, 0.979x the T1
figure) and 399.0 MiB after the click, so the shell itself did not grow; the whole-tree figures above are
what the harness prints for this method.

The T6 binary is `$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri`, sha256
`7c176900e79d179d1eedc9282b13820a677a106601e46e8ef793511bb4ff68e2`, built from branch `t6-proof` after master
(T16 bundled payload, T17 Windows compile) was merged in. Both bundles come from the same
`cargo tauri build --bundles deb,appimage` step; the AppImage was also launched once with
`APPIMAGE_EXTRACT_AND_RUN=1` and reached the app's first painted frame. The RAM and startup figures above
were measured on the pre-merge T6 binary (sha256 `2893f25b...`, 24,502,648 bytes); the merge adds about 2%
to the binary size and touches the payload-install path, which does not run in attach mode, so those figures
are kept and the sizes are the ones from this build.

Backend (Python) processes excluded from every RAM figure: 0 in every run (attach mode, see below).
Electron's two `linux-unpacked` trees (`hermes-agent-integrated` and `hermes-final-merge2`) both measure
335.5 MB apparent; `final-merge2` has one extra file (435).

Run 1 idle, by command-line type (MiB PSS): browser process 104.9, utility processes 48.4, zygote and
renderer processes 261.0. Run 2 after the click: browser 110.4, utility 47.8, zygote and renderer 237.3.
Forked renderers keep their zygote's command line, so "zygote" includes renderers.

Run-to-run spread on idle RAM is about 5% (414.3 vs 436.8 MiB). Treat differences under that as noise.

## Startup profile (T12)

The startup figure above is *first contentful paint*, and for both shells that content is the **remote
Hermes web app** served by the test backend, not a local shell page. Measured back to back on the same
minute (both shells under `dbus-run-session --`, launch link via `HERMES_TAURI_URL`, fresh dir per run, load
recorded; no build lock, which was held by other jobs):

| set | runs (ms) | median (ms) | load before -> after |
|---|---|---|---|
| Tauri (T1 binary) | 6944.1, 10021.5, 9543.8 | 9543.8 | 5.76 -> 8.18 |
| Electron (v0.17.0) | 5929.7, 2995.5, 3244.1 | 3244.1 | 8.18 -> 11.07 |

Median ratio 2.94x. The cost is **not** shell startup: instrumenting map time versus paint time shows the
Tauri window maps in ~0.46 s but stays a single flat colour until ~6.2 s with the launch URL, whereas with
**no** launch URL (local splash only) the same binary paints at **1.75 s** (runs 1862.5, 870.1, 1750.2). The
shell navigates the webview to the remote app (`src-tauri/src/connect.rs:177`) before the local splash has
painted, so the first frame anyone sees is the remote app. `WebKitWebProcess` is CPU-saturated (~1 core) for
that whole gap; an `strace -f -tt -T` of one cold start found no syscall gap over 386 ms. DRI3 fails under
Xvfb so Mesa renders with llvmpipe, and the shell forces `WEBKIT_DISABLE_DMABUF_RENDERER=1`; the numbers are
software-rendered.

Per-fix medians (3 cold runs each, launch URL, fresh dirs; load average rose 6.3 -> 14.6 through the matrix,
and the spread tracks the load, not the variable):

| variable tried | paint median (ms) | load |
|---|---|---|
| baseline (nothing extra) | 6110.5 | 6.30 |
| `NO_AT_BRIDGE=1` | 5437.7 | 7.54 |
| `GTK_A11Y=none` | 5256.8 | 9.18 |
| `GDK_DEBUG=` (empty) | 6818.7 | 9.29 |
| `GIO_USE_VFS=local` | 6402.4 | 11.87 |
| `GTK_USE_PORTAL=0` | 5701.3 | 14.63 |
| (exploratory) `WEBKIT_DISABLE_COMPOSITING_MODE=1` | 5982.5 | 12.22 |
| pre-warmed fontconfig cache (dir not wiped) | 6160.7 | ~10 |

None of these changed first paint outside noise (all 5.2-6.8 s), the two "best" values ran at the lowest
load and the "worst" at a higher load, and the app wrote no fontconfig cache into `XDG_CACHE_HOME/fontconfig`
at all (only `mesa_shader_cache`). With the harness's fresh-dir wipe disabled entirely the Tauri median was
4442 ms (runs 4442, 1688, 6155) — high variance, and the 1688 ms run shows the first frame is a race between
the local splash and the remote app depending on which composites first. See `.oc/reports/T12.md` for the
full profile, fix matrix and harness change.

## Splash-first fix (T6)

T6 fixed the blank window T12 diagnosed, in the shell: the window now shows the shell's own boot page
(the *splash*, a second window with the same title, always on top) until the app's page reports its first
painted frame, and the main window is only navigated to the app's URL underneath it. Measured with the
same harness and method, 3 cold runs each, fresh dir per run, `dbus-run-session --` wrapper, load
recorded:

| set | runs (ms) | median (ms) | what the clock stops at | load before -> after |
|---|---|---|---|---|
| T6, first painted window | 2024.7, 2839.1, 1834.8 | 2024.7 | the splash: the first window with real content | 14.95 -> 15.03 |
| T6, remote UI painted | 7328.3, 6234.3, 6443.0 | 6443.0 | the app's own `First frame of the Hermes app painted` log line | 15.03 -> 13.84 |

The first figure is what a user sees: **2.0 s instead of 6.7 s** (T1) and instead of a flat single colour
for the whole gap. The second figure is the honest time to the remote UI, unchanged in kind from T12's
~6-10 s: 0.96x T1's 6725.4 ms and 0.67x T12's back-to-back 9543.8 ms, both measured under different
load. The gap between the two rows is the app's JavaScript bundle executing in JavaScriptCore (T14: FCP
~4.4 s in WebKitGTK vs ~2.8 s in Chromium, ~3.2 s of it JS after `domInteractive`).

An earlier set of the same two measurements on the same binary at load 10.5 gave 2940.1 ms and 6328.7 ms,
so the splash figure moves with the machine (2.0-2.9 s) while the remote-UI figure is stable (6.3-6.4 s).
The splash number is not a race the shell can lose: the splash is a separate always-on-top window, so it
is on screen as soon as WebKit paints it, whatever the app's page is doing behind it.

The harness measures the first painted window with `--stop-log` for the second row, because the window
check cannot tell the splash from the app behind it: both are 1280x820 with the same title. `--stop-log`
stops at the first line the command writes to `--log` matching a regex instead of at the first painted
window; it is the only helper change this task made beyond accepting Xvfb's 24-bit-per-pixel root capture
(see Caveats).

## WebKit render cost (T14)

T14 investigated where WebKitGTK spends the ~5 s gap between window mapping (~0.5 s) and the first frame
of the remote Hermes web app (~6–10 s), evaluating whether Skia software rasterization, Mesa llvmpipe GL,
or JavaScript execution dominates, and whether shell-level environment variables can reduce it.

### Startup matrix (interleaved baselines)

Measured with the frozen T1 binary (`$HOME/.cache/hermes-tauri/bin-t1/hermes-tauri`) under `dbus-run-session --`
and the standard harness (`scripts/measure.sh startup`, 3 cold runs per variant, fresh dir wiped per run,
launch link passed via `HERMES_TAURI_URL`). Baseline sets were interleaved between each variant to separate
variable effects from machine load drift:

| Set / Variable | Runs (ms) | Median (ms) | Load before -> after |
|---|---|---|---|
| Baseline 1 | 6151.8, 8155.3, 4770.5 | 6151.8 | 4.19 -> 4.02 |
| `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` | 6856.4, 8431.5, 9141.5 | 8431.5 | 4.02 -> 7.93 |
| Baseline 2 | 8468.5, 8325.8, 6453.2 | 8325.8 | 7.93 -> 8.89 |
| `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` + `WEBKIT_SKIA_CPU_PAINTING_THREADS=4` | 6155.8, 6371.6, 6236.5 | 6236.5 | 8.89 -> 9.55 |
| Baseline 3 | 6294.4, 6271.7, 5136.8 | 6271.7 | 9.55 -> 9.43 |
| `LIBGL_ALWAYS_SOFTWARE=1` | 6186.8, 6041.3, 7337.5 | 6186.8 | 9.43 -> 9.87 |
| Baseline 4 | 6563.8, 8177.7, 8587.1 | 8177.7 | 9.87 -> 9.22 |
| `JSC_useJIT=false` | 9570.3, 9178.7, 7330.7 | 9178.7 | 9.22 -> 8.61 |
| Baseline 5 | 6195.6, 6292.2, 6339.5 | 6292.2 | 8.61 -> 7.96 |

Both `WEBKIT_SKIA_ENABLE_CPU_RENDERING` and `WEBKIT_SKIA_CPU_PAINTING_THREADS` exist in WebKitGTK 2.52
(`libwebkit2gtk-4.1-0` 2.52.6, introduced in Igalia's Skia integration). However, neither variable yields
a benefit: `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` (8431.5 ms) matches its adjacent baseline (8325.8 ms), and
adding 4 painting threads (6236.5 ms) matches baseline 3 (6271.7 ms). `LIBGL_ALWAYS_SOFTWARE=1` (6186.8 ms)
shows that DRI3 failure probing is not an overhead source. Meanwhile, `JSC_useJIT=false` slows startup to
9178.7 ms (+1.0–2.9 s over baselines), confirming that JIT is active and that JS execution speed is critical.

### Page-side timing (WebKitGTK vs Chromium)

Page-level instrumentation was captured in a 1280x820 viewport under the same Xvfb display to isolate network,
DOM parsing, JavaScript compilation/execution, and paint milestones (3 runs each):

| Metric | WebKitGTK (default) | WebKitGTK (Skia CPU) | Chromium (Playwright) |
|---|---:|---:|---:|
| `responseEnd` (ms) | 19.0 (19.0, 22.0, 19.0) | 22.0 (22.0, 38.0, 22.0) | 25.9 (76.8, 17.6, 25.9) |
| `domInteractive` (ms) | 414.0 (370.0, 460.0, 414.0) | 531.0 (531.0, 642.0, 382.0) | 369.1 (371.4, 317.0, 369.1) |
| `domContentLoadedEventEnd` (ms) | 1181.0 (1054.0, 1218.0, 1181.0) | 1305.0 (1305.0, 2065.0, 1045.0) | 1767.7 (2191.3, 1735.0, 1767.7) |
| `loadEventEnd` (ms) | 1187.0 (1060.0, 1227.0, 1187.0) | 1313.0 (1313.0, 2075.0, 1050.0) | 2802.5 (3662.1, 2691.9, 2802.5) |
| `first-paint` (ms) | none | none | 372.0 (428.0, 332.0, 372.0) |
| `first-contentful-paint` (ms) | 4364.0 (4364.0, 4921.0, 4283.0) | 4569.0 (4513.0, 5651.0, 4569.0) | 2808.0 (3712.0, 2704.0, 2808.0) |
| Longest `longtask` (ms) | not supported | not supported | 944.0 (1383.0, 871.0, 944.0) |
| Total resource transfer (KB) | 290.6 (290.6, 290.6, 290.3) | 290.3 (290.3, 290.6, 290.3) | 9172.4 (9172.4, 9173.9, 9172.4) |
| Total decoded size (KB) | 241.6 (241.6, 241.6, 241.6) | 241.6 (241.6, 241.6, 241.6) | 9100.5 (9100.5, 9101.9, 9100.5) |
| Resource count | 250 (250, 250, 250) | 250 (250, 250, 250) | 250 (250, 250, 250) |

WebKitGTK does not implement the Long Tasks API (`PerformanceObserver.supportedEntryTypes` does not include
`longtask`). In Chromium, long tasks during bundle initialization reach 944–1383 ms.

### Conclusion

1. **The gap is JavaScript execution and bundle initialization, not rasterization/compositing.** Network and
   initial DOM parsing finish in ~1.18 s. WebKit spends the next ~3.2 s parsing, compiling, and running the
   React application scripts before reaching `first-contentful-paint` (~4.36 s in WebKit vs ~2.81 s in Chromium).
2. **Skia CPU rasterization does not fix it.** Forcing CPU tile rendering (`WEBKIT_SKIA_ENABLE_CPU_RENDERING=1`)
   does not improve FCP (4569 ms vs 4364 ms) or shell startup (8431.5 ms vs 8325.8 ms baseline). Mesa llvmpipe
   software GL is not the bottleneck; the bottleneck precedes tile painting.
3. **No environment variable provides a win.** The only effective remedy is architectural: keep the local shell
   splash screen visible until the remote webview signals its first frame is ready, providing an immediate
   ~1.7 s visual feedback instead of a 6 s blank window.

## Method

- **Size**: a walk with `lstat`, no symlinks followed. Apparent size is the sum of `st_size`; on-disk size
  is `st_blocks * 512`. MB here means MiB (1,048,576 bytes).
- **RAM**: the command runs under `xvfb-run -a -s "-screen 0 1440x900x24"` in its own session. The shell
  tree is every descendant of the command plus every process in its session, minus the `xvfb-run`
  wrapper and the X server. PSS and RSS come from `/proc/<pid>/smaps_rollup`. Each figure is the median of
  5 samples taken 2 s apart, after a 20 s settle. Processes matching `--pid-pattern` are reported as
  backend and left out of the shell total. TCP sockets of the tree are recorded from `ss -tanp` at the
  last sample. At the end the tree gets TERM, then KILL after 5 s, then session cleanup; the run fails if
  anything survives.
- **Navigation**: `--click-window X,Y` clicks inside the main window and measures again. The click
  coordinates are relative to the window's top-left corner.
- **Startup**: the clock starts when the command's pid is written, just before `exec`. It stops at the
  first visible window at least 200x200 px whose 32x20 sample grid of its XWD capture holds two or more
  colours. The user-data directory is wiped before each run. This measures first paint, not the moment
  the UI finished loading data. If the command is a wrapper that forks the X11 client (e.g.
  `dbus-run-session --`, which the T12 and T6 runs use), the window is found by searching the wrapper's
  process tree, because the wrapper's pid is not the window's `_NET_WM_PID`.
- **Startup, `--stop-log REGEX`**: stops at the first line the command writes to `--log` that matches the
  regex instead of at the first painted window. Needed where the window is not the milestone: the T6
  shell shows its splash first and only later the app, and both are the same size and title. The clock is
  the same; only the stop condition changes, and the JSON record says so in `clock` and `painted`.
- **XWD depths**: `xwd` reports the Xvfb root at 24 bits per pixel while any window is mapped and 32
  while none is, and a window's own capture at 32. All of them store four bytes per pixel
  (`bytes_per_line == width * 4`), so the parser accepts both and rejects anything whose stride does not
  match, instead of failing on the root capture.
- **Environment for attach mode**: the command gets exactly the variables in an env file (mode 600,
  never printed): a throwaway `HOME` and `XDG_*` under `/tmp`, `HERMES_HOME` pointing at the test Hermes
  state, `HERMES_DESKTOP_USER_DATA_DIR`, `HERMES_DESKTOP_REMOTE_URL` and `HERMES_DESKTOP_REMOTE_TOKEN`
  for the test server, `HERMES_DESKTOP_IGNORE_EXISTING=1`, `HERMES_DESKTOP_SKIP_QUIT_CONFIRM=1` and
  `ELECTRON_DISABLE_SANDBOX=1`. The command line is
  `Hermes --no-sandbox --password-store=basic --proxy-server=http://127.0.0.1:1 --user-data-dir=<dir>`.
- **Egress**: `--proxy-server=http://127.0.0.1:1` sends all Chromium traffic to a closed port, so it
  fails closed. Without it the shell opened external HTTPS connections (background Chromium traffic). Any
  Tauri measurement needs the same socket check, with WebKit's traffic counted.
- **Test server**: `test-hermes.sh` from the T2 worktree, copied with the state directory set to
  `/tmp/hermes-tauri-test-t9` and port 18800. The seed configures no model provider, so the onboarding
  gate hides the UI. The measurement state adds `OPENAI_BASE_URL` pointing at unused loopback port 18899
  and a placeholder key to the test state's `.env`. Nothing listens on 18899. This is a gap for T2 to
  close in its seed.

## Commands

```sh
scripts/measure.sh size --label electron-linux-unpacked <path>/linux-unpacked
scripts/measure.sh ram --label electron-agent-integrated --env-file <env> \
  --pid-pattern '(^|/)python[0-9.]*( |$)|hermes_cli|tui_gateway' \
  --settle 20 --samples 5 --screenshot <png> \
  -- <path>/linux-unpacked/Hermes --no-sandbox --password-store=basic \
     --proxy-server=http://127.0.0.1:1 --user-data-dir=<dir>
scripts/measure.sh ram ... --click-window 68,100 -- <same command>
scripts/measure.sh startup --label electron-agent-integrated --env-file <env> --runs 3 \
  --fresh-dir <dir> -- <same command>
```

Run the RAM and startup commands under `flock /tmp/hermes-build.lock` so builds by other jobs don't
distort the numbers. `<env>` holds the launch link and must not be printed or committed.

## Machine

- CPU: Intel Xeon W-2135 @ 3.70 GHz, 8 logical CPUs
- RAM 76 GiB, swap 7 GiB
- Kernel 7.0.0-34-generic
- GPUs: two AMD Radeon cards are present (Navi 24 PRO W6400, Ellesmere WX 5100). Xvfb has no GPU
  acceleration, and no GPU process appeared in the measured Electron tree.
- Xvfb 2:21.1.22-1ubuntu1, xdotool 1:3.20160805.1-5.1build1, xwd from x11-utils 7.7+7build1,
  iproute2 6.19.0
- Electron 40.10.2 (`linux-unpacked`, app v0.17.0, August build); backend v0.22.0 as shown in the UI
- WebKitGTK 2.52.6 (`libwebkit2gtk-4.1-0` 2.52.6-0ubuntu0.26.04.1)
- The machine was shared: other sessions' browsers and X servers were running during the runs.
- Software rendering only: under Xvfb, WebKit's EGL reports `DRI3 error` and falls back to llvmpipe; the
  measured shells are not GPU-accelerated even though AMD GPUs are present. `kernel.perf_event_paranoid=4`
  and `kernel.yama.ptrace_scope=1` on this host block `perf` and `gdb` attach.

## Tauri

The T1 measurements used T1's frozen binary (`$HOME/.cache/hermes-tauri/bin-t1/hermes-tauri`, sha256 `eb7bbd113933554a...`), in attach mode (`--url <launch link>`). Tray, hotkey, and deep-link features are not included in that build (introduced in T5).

The T6 measurements below used the T6 binary, in attach mode, under the T12 wrapper form
`dbus-run-session -- <binary>`. `HERMES_TAURI_URL` in the env file carries the launch link; the CLI
`--url` flag was not used because the env file is how the T12 method passes it.

### Commands

```sh
# T1 (history)
scripts/measure.sh size --label tauri-binary $HOME/.cache/hermes-tauri/bin-t1/hermes-tauri
scripts/measure.sh ram --label tauri-t1-attach \
  --pid-pattern '(^|/)python[0-9.]*( |$)|hermes_cli|tui_gateway' \
  --settle 20 --samples 5 --click-window 16,370 \
  -- $HOME/.cache/hermes-tauri/bin-t1/hermes-tauri --url "$LAUNCH_URL"
scripts/measure.sh startup --label tauri-t1-attach --runs 3 \
  --fresh-dir /tmp/tauri-startup-fresh \
  -- $HOME/.cache/hermes-tauri/bin-t1/hermes-tauri --url "$LAUNCH_URL"

# T6
BIN=$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri
scripts/measure.sh size --label tauri-t6-binary "$BIN"
scripts/measure.sh size --label tauri-t6-deb "$HOME/.cache/hermes-tauri/target-T6/release/bundle/deb/Hermes Tauri_0.1.0_amd64.deb"
scripts/measure.sh size --label tauri-t6-appimage "$HOME/.cache/hermes-tauri/target-T6/release/bundle/appimage/Hermes Tauri_0.1.0_amd64.AppImage"
scripts/measure.sh ram --label tauri-t6-attach --env-file <env> \
  --pid-pattern '(^|/)python[0-9.]*( |$)|hermes_cli|tui_gateway' \
  --settle 20 --samples 5 --screenshot <png> \
  -- dbus-run-session -- "$BIN"
scripts/measure.sh ram ... --click-window 16,355 -- dbus-run-session -- "$BIN"
scripts/measure.sh startup --label tauri-t6-firstpaint --env-file <env> --runs 3 \
  --fresh-dir /tmp/hermes-tauri-t6-measure/xdg --log <log> \
  -- dbus-run-session -- "$BIN"
scripts/measure.sh startup --label tauri-t6-remoteui --env-file <env> --runs 3 \
  --fresh-dir /tmp/hermes-tauri-t6-measure/xdg --log <log> \
  --stop-log 'First frame of the Hermes app painted' \
  -- dbus-run-session -- "$BIN"
```

### Process breakdown and egress

- T1 idle PSS breakdown (median run): `hermes-tauri` 77.3 MiB, `WebKitNetworkProcess` 25.1 MiB, `WebKitWebProcess` 287.7 MiB (total 390.2 MiB PSS, 571.5 MiB RSS).
- T1 after click to `#/team` (median run): `hermes-tauri` 77.4 MiB, `WebKitNetworkProcess` 25.1 MiB, `WebKitWebProcess` 289.2 MiB (total 391.8 MiB PSS, 573.1 MiB RSS).
- T6 idle PSS breakdown (run 1 / run 2): `hermes-tauri` 72.4 / 72.2 MiB, `WebKitNetworkProcess` 23.6 / 23.5 MiB, `WebKitWebProcess` 288.8 / 283.5 MiB. The rest of the tree is the session bus, the accessibility bus and the XDG portal helpers (`xdg-desktop-portal-gtk` 4.7 / 4.9, `glycin-image-rs` 4.7 / 4.6, `xdg-desktop-portal` 4.3 / 4.3, `dbus-daemon` 1.2, `gvfsd` 1.1, `at-spi2-registryd` 0.9, and smaller ones), about 20 MiB PSS together.
- T6 after the click to `#/team`: `hermes-tauri` 78.1 MiB, `WebKitNetworkProcess` 24.1 MiB, `WebKitWebProcess` 296.8 MiB.
- T6 egress check: `{"external_connections": 0, "listening": {}, "loopback_peers": {"127.0.0.1:18800": 3}}` — zero external network connections; only local loopback to the test Hermes daemon. WebKit's traffic is counted the same way as Chromium's.
- Backend processes: 0 (attach mode).

## Caveats

- Startup is first paint, not data loaded (see Method). For the Electron column and the T1 column the first
  painted content is the remote Hermes web app, so the ratio mixes shell overhead with web-engine rendering
  (WebKitGTK here vs Chromium). For the T6 column there are two figures: the splash (2.9 s, the shell's own
  page) and the remote UI (6.3 s, the app). Shell-to-remote-UI is the figure to compare with Electron.
- The Electron and Tauri columns were measured about three hours apart, and the Tauri runs overlapped heavy
  builds from other jobs on the same machine. The back-to-back runs in the Startup profile section
  (Tauri 9543.8 ms vs Electron 3244.1 ms) confirm the ~2.8-2.9x ratio under one load window; the absolute
  numbers are still affected by the shared machine (load 6-14 on 8 CPUs).
- RAM after the click covers the Messaging page for Electron (`68,100`), whereas for Tauri it navigates to the Team page `#/team` (`16,370`) via the activity rail.
- Attach mode spawned no backend in any run, so the backend column is 0. The pattern is a guard, not a
  measurement of backend cost; the backend is the same for both shells.
- The `by_kind` grouping uses the command-line `--type` value for Electron. For Tauri, processes are grouped by executable name (`hermes-tauri`, `WebKitNetworkProcess`, `WebKitWebProcess`).
- The T6 startup "first painted window" is the splash, and its clock stops when the harness first sees a
  window with content; the `--stop-log` row stops when the shell logs the app's first frame. Neither is a
  compositor-level callback, and both run under software rendering (llvmpipe) on a shared machine at load
  15, so treat the absolute milliseconds as an upper bound; the 3.3x improvement over T1's same-harness
  figure is the meaningful part.
- The T6 runs overlap other jobs' builds and X servers (load 3.6-29.1 on 8 CPUs during the sets above).
  The T1 and Electron columns were measured hours earlier under a different load.
- Two T6 RAM runs, not three: the lock is shared and each RAM run holds it for the full settle and sample
  window. The two idle figures differ by 3% (409.3 vs 396.7 MiB), so the median is quoted with both runs.
- The T6 bundle sizes are from this task's own `cargo tauri build --bundles deb,appimage` in
  `$HOME/.cache/hermes-tauri/target-T6`, not from the shared `target/`. They were re-measured after the
  T16/T17 merge (the binary grew 24,502,648 -> 25,008,328 bytes, +2.1%), which is under the 5% threshold
  for re-running the RAM and startup sets, so only the sizes were refreshed.
- Electron's AppImage (142 MB) and deb (114 MB) figures are still from an earlier build and were not
  re-measured; the deb ratio in particular is approximate.
- `scripts/measure-helper.py` was extended in this task with `--stop-log` and with 24-bit-per-pixel XWD
  support. The Electron and T1 numbers were produced with the older parser; the change only relaxes a
  format check (the 32-bit path is byte-identical), so those numbers stand.
