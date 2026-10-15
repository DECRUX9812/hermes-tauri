# Tauri Shell Proof Screenshots (early proof, T1 build)

Screenshots captured under Xvfb (1440x900x24) with a private D-Bus session running the frozen T1 binary (sha256 `eb7bbd113933...`) in
attach mode against an isolated test Hermes on a port in 18800–18899. The PNGs live in `~/Videos/tauri-shots/` and are not committed.
These are the early shots; the final T5 build replaces them (see the later sections when present).

## Image list

- `01-boot.png`: the boot page while connecting ("Connecting to Hermes…", checking the test port) before the test Hermes was up. The
  1280x820 window sits in the top-left of the 1440x900 root; the black border is the bare Xvfb root.
- `02-sessions.png`: `#/` with the six seeded demo sessions in the sidebar and three of them as recent-task cards. The composer shows
  the test Hermes working directory's git branch as a chip.
- `03-plugin-rail.png`: the plugin rail on the far left with the Kanban plugin open (`#/kanban`, empty default board).
- `04-team.png`: `#/team` with the Demo Product Team org chart (Research Bot as lead, Writer Bot, Reviewer Bot).
- `05-group-chat.png`: a group chat created through the UI with demo bots (Reviewer Bot, Writer Bot) and one neutral typed message ("Demo: please summarize the launch checklist") submitted to the room. The seeded "Demo Product Room" does not hydrate in a fresh profile (docs/test-hermes.md), so the proof creates a fresh room via the UI.
- `06-reload-before.png`: the group chat room before the F5 reload.
- `07-reload-after.png`: the same route after F5 reload: the webview reloaded and repainted, still attached.

## Reproducing the proof (T5 build, `scripts/proof.sh`)

The shots above were captured by hand from the early T1 build. The current proof is one lock-free driver: it starts a disposable test
Hermes, re-executes itself under a private D-Bus session and Xvfb, drives the shell with real input only, and writes the eight shots
plus an index into the output directory.

```sh
scripts/proof.sh [--bin PATH] [--out DIR] [--port-dir DIR] [--spawn-shutdown]
```

- `--bin PATH` shell binary to drive (default `$HOME/.cache/hermes-tauri/target/release/hermes-tauri`).
- `--out DIR` shots, `README.md`, one log per shell launch and `dbus-stderr.log` (default `/tmp/hermes-tauri-t6g/shots`).
- `--port-dir DIR` state dir of the disposable test Hermes (default `/tmp/hermes-tauri-test-proof`). It must live under
  `/tmp/hermes-tauri-test*`, so the proof can never talk to a real Hermes.
- `--spawn-shutdown` skips the shot list: the shell starts its own Hermes from `HERMES_TAURI_CMD`, then receives SIGTERM and the
  script proves by recorded pid and pgid that no spawned backend survived.

Everything runs from a scratch working directory, the shell's XDG dirs are redirected under `--out` (fresh webview profile per run,
nothing written to the real home), and the window is placed at 0,0 sized 1280x820 on the 1440x900 root, so the black margin is the bare
Xvfb root. Input is real: `xdotool` keys and clicks plus the shell's own deep links. Nothing is injected into the page and no
WebDriver or devtools is used. `xvfb-run`, `dbus-run-session`, `scrot`, `xdotool`, `xwd` and `python3` must be installed;
`ImageMagick` is not required.

Output lines:

- `PASS <shot>` the shot was written, its frame was painted (more than 12 distinct colours on a 32x20 sample grid) and its md5 had not
  appeared earlier in the run.
- `FAIL <shot>: why` the shot was written and rejected, with the reason; the run continues through the rest of the list.
- `proof: ...` progress: the ports it picked, the shell pid and log file, and what it is waiting for.

The run exits non-zero if any shot failed, and it never leaves a stale PNG behind: the output directory is cleared at the start and
every file it reports is written by that run. Waits are driven by real signals (window mapped, the shell's own stderr log lines such as
`Navigated to`, a painted frame, the loading state that follows a reload) rather than fixed sleeps alone.
