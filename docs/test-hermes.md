# Disposable test Hermes

`scripts/test-hermes.sh start` starts the pinned Desktop webapp with neutral demo data. By default,
all runtime state is isolated under `/tmp/hermes-tauri-test`. The script selects a free loopback
port in `18800–18899`, waits for `/api/health`, and prints only the server origin.

Set `HERMES_TAURI_TEST_DIR` to use a separate instance directory, for example:

```sh
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh start
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh status
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh stop
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh reset
```

The override controls the pid, launch link, logs, and all runtime directories. It must resolve to
`/tmp/hermes-tauri-test`, a sibling whose name starts with `/tmp/hermes-tauri-test-`, or a child
directory beneath `/tmp/hermes-tauri-test`. `reset` stops only the managed server and removes only
the selected directory. Each instance checks the listening ports before choosing one, so a running
instance is skipped by the next start.

Use `status` to check health, `url` to retrieve the private launch link, and `cmd` to print the
shell-escaped `HERMES_TAURI_CMD` assignment. The launch link is stored with mode `0600`; do not
paste it into logs or reports. `verify-isolation` inspects every process in the managed server
group, prints only `HOME`, `HERMES_HOME`, and `HERMES_RUNTIME_DIR`, and lists any open descriptors
under the real Hermes home with their access mode and a relative path. It fails on unexpected
environment values or any writable descriptor there; read-only interpreter/library descriptors
are allowed.

The script uses the Python environment selected by the existing Hermes launcher. Set
`HERMES_TAURI_PYTHON` to an existing Hermes-compatible interpreter if automatic discovery does
not apply. It does not install or alter Python dependencies.

The isolated configuration sets `onboarding.run: false`, which disables the backend onboarding
flow but does not dismiss the first-run provider picker. The picker checks runtime readiness or
the browser-local skip flag `hermes-onboarding-skipped-v1`; the fresh-browser proof clicks
“I'll choose a provider later.” No provider key is needed.

The six seeded chats appear in the Sessions sidebar. The seeded Bot Team and Demo Product Room are
available at `#/team`. The room transcript is written through Hermes's `merge_ui_meta` API as a
version-3 `hermes-bots-groups` projection on the default profile. The plugin loads its local room
cache, then pulls `profiles.list` before publishing, so a fresh browser can hydrate the four
server-seeded messages without local-storage injection. The fresh-profile screenshot is
`/tmp/hermes-tauri-t2/group-chat-fresh.png`.

The plugin rail is served by the pinned webapp and can be checked with
`/api/desktop-plugins/list`. The isolated setup returned zero desktop-plugin entries; the built-in
app navigation still rendered.
