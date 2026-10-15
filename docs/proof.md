# Tauri Shell Proof Screenshots (final proof, T6 build)

Screenshots captured under Xvfb (1440x900x24) with a private D-Bus session running the T6 binary
(`$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri`, sha256 `0c2ac2007b36bf0d9aeea3efc2699f4d5a6e97a73a22e4605601910b1bb92545`)
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

### The final run

One run of `scripts/proof.sh --bin $HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri --out ~/Videos/tauri-shots`:

| shot | result | colours | md5 | what it shows |
|---|---|---|---|---|
| 01-boot.png | PASS | 13 | ffaf880ddc80962602da94957eb42ee1 | boot page while Hermes is unreachable: launched against the closed port 18802, the page reports it and offers Retry |
| 01b-splash.png | PASS | 9 | 419269494a0b5aee7bc3027efc9a38c7 | the splash (the boot page) still in front of the window while the Hermes app loads, before its first frame |
| 02-sessions.png | PASS | 21 | 56122dacb9c5c19cec0ae57ae0375187 | home route after attaching: the seeded demo session titles in the sidebar and the seeded recent tasks on the page |
| 03-plugin-rail.png | PASS | 16 | 0d64376e30c78b8665cc61261f9a8078 | the Kanban plugin page reached by deep link, the plugin rail on the far left |
| 04-team.png | PASS | 26 | 5f71d55c0e8594b2c7b8d29d11fb6966 | the Demo Product Team page reached by deep link |
| 05-group-chat.png | PASS | 18 | 83e72bcce08752b95ee2b8ea0bb7539c | a group chat created through the UI with two demo bots and one submitted neutral message |
| 06-reload-before.png | PASS | 19 | 0db609271b199963fcc1355ba2f5a094 | the group chat view (in-app state on the #/team route) before the reload |
| 07-reload-after.png | PASS | 29 | ce7683a7ce77a39238581b406208e4f6 | the #/team route after F5: the webview reloaded, repainted from the backend and kept the session |
| 08-deeplink.png | PASS | 200 | d47a85f81f88d61a54b5b0eb1c9b56c6 | the same window after a second invocation handed it the /studio route |

| check | result | what it proves |
|---|---|---|
| reload-session | PASS | F5 re-navigated the webview to the same #/team route, the app painted again from the backend, no second window and no connection error: the session survived the reload |
| splash-cap | PASS | the cap removed the always-on-top splash while the app was still loading (same code path as the 30 s default, shortened to 3000 ms); one window was left and the app painted afterwards |

`--spawn-shutdown` passed in its own run: the shell started its own Hermes (pid 3806995, pgid 3806995) and SIGTERM to the shell
stopped it within 1 s, with no surviving pid or pgid.

## Reproducing the proof (T6 build, `scripts/proof.sh`)

One lock-free driver: it starts a disposable test Hermes, re-executes itself under a private D-Bus session and Xvfb, drives the shell
with real input only, and writes the nine shots plus an index into the output directory.

```sh
scripts/proof.sh --bin "$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri" --out ~/Videos/tauri-shots
scripts/proof.sh --bin "$HOME/.cache/hermes-tauri/target-T6/release/hermes-tauri" --out /tmp/hermes-tauri-t6-spawn --spawn-shutdown
```

- `--bin PATH` shell binary to drive (default `$HOME/.cache/hermes-tauri/target/release/hermes-tauri`).
- `--out DIR` shots, `README.md`, one log per shell launch and `dbus-stderr.log` (default `/tmp/hermes-tauri-t6g/shots`).
- `--port-dir DIR` state dir of the disposable test Hermes (default `/tmp/hermes-tauri-test-proof`). It must live under
  `/tmp/hermes-tauri-test*`, so the proof can never talk to a real Hermes.
- `--spawn-shutdown` skips the shot list: the shell starts its own Hermes from `HERMES_TAURI_CMD`, then receives SIGTERM and the
  script proves by recorded pid and pgid that no spawned backend survived. It clears `*.png` in `--out` like the shot run does, so
  give it its own directory if you want to keep a shot run's images.

Everything runs from a scratch working directory, the shell's XDG dirs are redirected under `--out` (fresh webview profile per run,
nothing written to the real home), and the window is placed at 0,0 sized 1280x820 on the 1440x900 root, so the black margin is the bare
Xvfb root. Input is real: `xdotool` keys and clicks plus the shell's own deep links. Nothing is injected into the page and no
WebDriver or devtools is used. `xvfb-run`, `dbus-run-session`, `scrot`, `xdotool`, `xwd` and `python3` must be installed;
`ImageMagick` is not required.

Output lines:

- `PASS <shot>` the shot was written, its frame was painted (12 or more distinct colours on a 32x20 sample grid, or 4 for the two
  boot-page shots 01-boot and 01b-splash, whose pages score lower) and its md5 had not appeared earlier in the run.
- `PASS <check>` a proof that has no screenshot: it read the shell's own log and the window list.
- `FAIL <shot|check>: why` the artefact was written and rejected, with the reason; the run continues through the rest of the list.
- `proof: ...` progress: the ports it picked, the shell pid and log file, and what it is waiting for.

The run exits non-zero if any shot or check failed, and it never leaves a stale PNG behind: the output directory is cleared at the
start and every file it reports is written by that run. Waits are driven by real signals (window mapped, the shell's own stderr log
lines such as `Navigated to` and `First frame of the Hermes app painted`, a painted frame, the loading state that follows a reload)
rather than fixed sleeps alone. `01b-splash` samples until the splash's own boot page is on screen, and re-checks after every capture
that two windows with the app title are still mapped and that the app has not reported its first frame: the app's first frame dismisses
the splash, so either signal means the moment has passed and the shot fails instead of passing with a picture of the app or of a white
window that has not painted yet.
