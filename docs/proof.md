# Tauri Shell Proof Screenshots (final proof, T6 build)

Screenshots captured under Xvfb (1440x900x24) with a private D-Bus session running the T6 binary
(`$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri`, sha256 `f905110a4636e8406b74b51ab13b66dccea0e961e2bed0fece265cc708b0c58f`)
in attach mode against an isolated test Hermes on a port in 18800–18899. The PNGs live in `~/Videos/tauri-shots/` and are not
committed. One `scripts/proof.sh` run produced all of them, plus two checks that read the shell's log and the window list.

## Image list

- `01-boot.png`: the boot page of the main window with Hermes unreachable (launched against a closed port 18801): it reports
  `Hermes at http://127.0.0.1:18801 is not responding` and offers Retry. The splash has already been dismissed, because a failed
  start leaves the main window showing this same page.
- `01b-splash.png`: the splash while the app loads. Taken after the launch navigation and before the app's first frame, with both
  windows on screen; it shows the shell's own boot page (`Hermes is ready / Opening the app…`) in front of the window, which is what
  proves there is no blank window during the 6 s the app's bundle needs.
- `02-sessions.png`: `#/` with the six seeded demo sessions in the sidebar and three of them as recent-task cards.
- `03-plugin-rail.png`: the plugin rail on the far left with the Kanban plugin open (`#/kanban`, empty default board).
- `04-team.png`: `#/team` with the Demo Product Team org chart (Research Bot as lead, Writer Bot, Reviewer Bot).
- `05-group-chat.png`: a group chat created through the UI with demo bots (Reviewer Bot, Writer Bot) and one neutral typed message
  ("Demo: please summarize the launch checklist") submitted to the room. The seeded "Demo Product Room" does not hydrate in a fresh
  profile (docs/test-hermes.md), so the proof creates a fresh room via the UI.
- `06-reload-before.png`: the group chat view before the F5 reload. The room is in-app state on the `#/team` route, so the window's
  URL is still `#/team`.
- `07-reload-after.png`: after F5 the webview re-navigated to `#/team` and repainted from the backend, still attached, and the Team
  page is what the reload really restores. The `reload-session` check reads the shell's log for that navigation and for the absence
  of a second window or a connection error, which is the proof that the session survived.
- `08-deeplink.png`: the same window after a second invocation handed it the `/studio` route.

Checks (no PNG; each is reported as PASS or FAIL next to the shots):

- `reload-session`: F5 produced a new navigation to the route the window was on, one window, no connection error.
- `splash-cap`: with `HERMES_TAURI_SPLASH_CAP_MS=3000` the shell removed the always-on-top splash on its own while the app was still
  loading, left one window, and the app painted afterwards. That is the same code path as the 30 s default, shortened so the removal
  can be observed at all (the app paints in 6-9 s, so the real cap is exactly the case that never happens).

## Reproducing the proof (T6 build, `scripts/proof.sh`)

One lock-free driver: it starts a disposable test Hermes, re-executes itself under a private D-Bus session and Xvfb, drives the shell
with real input only, and writes the nine shots plus an index into the output directory.

```sh
scripts/proof.sh --bin "$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri" --out ~/Videos/tauri-shots
scripts/proof.sh --bin "$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri" --out ~/Videos/tauri-shots --spawn-shutdown
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
- `PASS <check>` a proof that has no screenshot: it read the shell's own log and the window list.
- `FAIL <shot|check>: why` the artefact was written and rejected, with the reason; the run continues through the rest of the list.
- `proof: ...` progress: the ports it picked, the shell pid and log file, and what it is waiting for.

The run exits non-zero if any shot or check failed, and it never leaves a stale PNG behind: the output directory is cleared at the
start and every file it reports is written by that run. Waits are driven by real signals (window mapped, the shell's own stderr log
lines such as `Navigated to` and `First frame of the Hermes app painted`, a painted frame, the loading state that follows a reload)
rather than fixed sleeps alone. `01b-splash` also requires two windows with the app title to be mapped at the moment of the shot, so a
run where the splash was already gone fails the shot instead of passing it with a picture of the app.
