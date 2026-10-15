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
- `05-group-chat.png`: a group chat opened through the UI, with two neutral typed messages and Research Bot working. The seeded "Demo
  Product Room" is listed in the sidebar but its four seeded messages do not hydrate in a fresh profile (UI boot race, see
  docs/test-hermes.md). The open room is a second room named "Demo Product" created during the run, not the seeded one.
- `06-reload-before.png`: `#/team` before the reload attempt.
- `07-reload-after.png`: `#/team` after RELAUNCHING the shell with the same launch link. F5 and Ctrl+R do nothing in the T1 build
  (WebKitGTK has no default reload accelerator), so this shot does not prove an in-place reload.
