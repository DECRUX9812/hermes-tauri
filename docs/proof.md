# Tauri Shell Proof Screenshots (T6a Early Proof)

Screenshots captured under Xvfb (1440x900x24) with a private D-Bus session running the frozen T1 binary (`$HOME/.cache/hermes-tauri/bin-t1/hermes-tauri`, sha256: `eb7bbd113933...`) in attach mode against an isolated test Hermes daemon on port 18825 (`/tmp/hermes-tauri-test-t6a`).

All shots verified: 1440x900 resolution, non-blank, zero personal data, zero host paths, zero credentials.

## Image List

- `01-boot.png`: Unreachable boot screen ("Hermes is not reachable") displaying connection retry attempts when launched against port 18825 before the daemon was started.
- `02-sessions.png`: Home view (`#/`) showing the Sessions sidebar populated with all six seeded demo sessions and recent activity cards.
- `03-plugin-rail.png`: Activity rail visible on the far left with the Kanban board icon active, displaying the seeded Kanban plugin board (`#/kanban`).
- `04-team.png`: Team view (`#/team`) showing the Demo Product Team organizational chart with Research Bot (Lead), Writer Bot, and Reviewer Bot.
- `05-group-chat.png`: Demo Product Room in the group chat workspace with three bot participants and a submitted neutral user message ("Demo: please summarize the launch checklist") visible in the transcript (seeded messages do not hydrate in fresh profiles due to a known UI projection bug in `GroupChatWorkspace.tsx`, so the room was opened and messaged via real UI clicks and keyboard input; no mock backend model response expected).
- `06-reload-before.png`: Team view (`#/team`) with the Demo Product Team loaded prior to reload verification.
- `07-reload-after.png`: Team view (`#/team`) following relaunch with the identical launch URL; neither F5 nor Ctrl+R triggers a reload in WebKitGTK without custom accelerators (to be added in T5/T6), confirming session token persistence, state preservation, and zero re-authentication prompts or errors upon relaunch.
