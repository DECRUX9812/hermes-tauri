# Desktop shell measurements

Size, idle RAM and window startup for the Electron shell and the Tauri shell, measured the same way on
one machine. The harness is `scripts/measure.sh` (bash entry point) with `scripts/measure-helper.py`
(stdlib only, Python 3.14). Every measurement prints one JSON line to stdout and a summary to stderr.

## Results

| Metric | Electron (v0.17.0 build) | Tauri |
|---|---|---|
| Size, `linux-unpacked` (apparent) | 335.5 MB (351,822,445 bytes, 434 files) | |
| Size, `linux-unpacked` (on disk) | 336.5 MB | |
| Size, AppImage | 142 MB, from an earlier build, not re-measured | |
| Size, deb | 114 MB, from an earlier build, not re-measured | |
| Idle RAM, shell PSS median (run 1 / run 2) | 414.3 MiB / 436.8 MiB | |
| Idle RAM, shell RSS median (run 1 / run 2) | 876.2 MiB / 878.3 MiB | |
| Idle RAM, processes in shell tree | 7 | |
| RAM after sidebar click to Messaging page, PSS median | 395.5 MiB (RSS 836.3 MiB) | |
| Window startup, median of 3 cold runs | 2434.1 ms (runs: 2211.8, 2514.3, 2434.1) | |

Backend (Python) processes excluded from every RAM figure: 0 in every run (attach mode, see below).
Electron's two `linux-unpacked` trees (`hermes-agent-integrated` and `hermes-final-merge2`) both measure
335.5 MB apparent; `final-merge2` has one extra file (435).

Run 1 idle, by command-line type (MiB PSS): browser process 104.9, utility processes 48.4, zygote and
renderer processes 261.0. Run 2 after the click: browser 110.4, utility 47.8, zygote and renderer 237.3.
Forked renderers keep their zygote's command line, so "zygote" includes renderers.

Run-to-run spread on idle RAM is about 5% (414.3 vs 436.8 MiB). Treat differences under that as noise.

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
  the UI finished loading data.
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
- The machine was shared: other sessions' browsers and X servers were running during the runs.

## Tauri

The Tauri column is filled in by T6, using the same commands and the same environment. The shell must
attach to the same test server, and its socket check must show only the test port.

## Caveats

- Startup is first paint, not data loaded (see Method).
- RAM after the click covers the Messaging page only. Driving `#/team` needs a stable route to it, which
  the sidebar did not provide; the team page is not measured.
- Attach mode spawned no backend in any run, so the backend column is 0. The pattern is a guard, not a
  measurement of backend cost; the backend is the same for both shells.
- The `by_kind` grouping uses the command-line `--type` value. Forked renderers share their zygote's
  command line, so renderers are counted under "zygote".
