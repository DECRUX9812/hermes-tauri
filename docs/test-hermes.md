# Disposable test Hermes

`scripts/test-hermes.sh start` starts the pinned Desktop webapp with neutral demo data. By default,
all runtime state is isolated under `/tmp/hermes-tauri-test`. The script selects a free loopback
port in `18800–18899`, waits for `/api/health`, and prints only the server origin.

`HERMES_TAURI_PIN` sets the Hermes checkout to run. It must be an absolute path to a checkout that
contains `hermes` and a built `apps/desktop/dist-webapp`. When it is unset, the script uses a
`_hermes-pin` directory next to the repository root, which resolves only when the working directory
is a worktree beside `_hermes-pin`; every example below sets it explicitly. If it resolves to
nothing, `start` stops with an error that names the path it tried.

Set `HERMES_TAURI_TEST_DIR` to use a separate instance directory, for example:

```sh
export HERMES_TAURI_PIN="/path/to/hermes-checkout"
export HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-demo
scripts/test-hermes.sh start
scripts/test-hermes.sh status
scripts/test-hermes.sh stop
scripts/test-hermes.sh reset
```

The override controls the pid, launch link, logs, and all runtime directories. It must resolve to
`/tmp/hermes-tauri-test`, a sibling whose name starts with `/tmp/hermes-tauri-test-`, or a child
directory beneath `/tmp/hermes-tauri-test`. `reset` stops only the managed server and removes only
the selected directory. Each instance checks the listening ports before choosing one, so a running
instance is skipped by the next start.

Use `status` to check health, `url` to retrieve the private launch link, and `cmd` to print the
backend command as the JSON argv array that `HERMES_TAURI_CMD` expects, for example
`HERMES_TAURI_CMD="$(scripts/test-hermes.sh cmd 18810)" hermes-tauri`. The printed array holds no
launch link or session token. The launch link is stored with mode `0600`; do not paste it into logs
or reports. `verify-isolation` inspects every process in the managed server
group, prints only `HOME`, `HERMES_HOME`, and `HERMES_RUNTIME_DIR`, and lists any open descriptors
under the real Hermes home with their access mode and a relative path. It fails on unexpected
environment values or any writable descriptor there; read-only interpreter/library descriptors
are allowed.

The script uses the Python environment selected by the existing Hermes launcher. Set
`HERMES_TAURI_PYTHON` to an existing Hermes-compatible interpreter if automatic discovery does
not apply. It does not install or alter Python dependencies.

## State-dir override and concurrency proof

Two independent instances can run concurrently on separate state directories and separate ports:

```sh
export HERMES_TAURI_PIN="/path/to/hermes-checkout"

# Start instance 1 (default state dir)
scripts/test-hermes.sh start 18800
# -> http://127.0.0.1:18800

# Start instance 2 (override state dir)
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh start 18801
# -> http://127.0.0.1:18801

# Both report running independently:
scripts/test-hermes.sh status
# -> running http://127.0.0.1:18800
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh status
# -> running http://127.0.0.1:18801

# Isolation verification on both passes cleanly:
scripts/test-hermes.sh verify-isolation
# pid=2442663 HOME=/tmp/hermes-tauri-test/home HERMES_HOME=/tmp/hermes-tauri-test/hermes HERMES_RUNTIME_DIR=/tmp/hermes-tauri-test/runtime
# pid=2442663 fd=3 mode=read-only path=tools/python-3.14.7+202****0901-linux-x64/bin/python3.14
# isolation=pass

HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh verify-isolation
# pid=2446989 HOME=/tmp/hermes-tauri-test-t2b/home HERMES_HOME=/tmp/hermes-tauri-test-t2b/hermes HERMES_RUNTIME_DIR=/tmp/hermes-tauri-test-t2b/runtime
# pid=2446989 fd=3 mode=read-only path=tools/python-3.14.7+202****0901-linux-x64/bin/python3.14
# isolation=pass

# Stopping instance 2 leaves instance 1 untouched:
HERMES_TAURI_TEST_DIR=/tmp/hermes-tauri-test-t2b scripts/test-hermes.sh stop
scripts/test-hermes.sh status
# -> running http://127.0.0.1:18800
```

## Provider picker suppression

The UI onboarding flow checks whether any runtime provider is configured:
- `apps/desktop/src/fork/store/onboarding.ts:240-275` (`checkRuntime`) calls `setup.status` and `setup.runtime_check`.
- `apps/desktop/src/fork/store/onboarding.ts:730-745` (`hasConfiguredProvider`) checks `state.runtimeReady || state.configured || ...`.
- `apps/desktop/src/fork/components/onboarding-index.tsx:340-355` mounts `<ProviderPicker />` only if `!hasConfiguredProvider(state)` and the local skip flag `hermes-onboarding-skipped-v1` is absent.

On the backend:
- `tui_gateway/methods_config.py:383-424` handles `setup.status`, returning `provider_configured: True` when `hermes_cli/main.py:1092-1110` (`_has_any_provider_configured`) finds a configured provider.
- `tui_gateway/methods_config.py:425-450` handles `setup.runtime_check`, delegating to `hermes_cli/runtime_provider.py:994-1065` (`resolve_runtime_provider`).
- For `provider: custom` with `api_key: no-key-required`, `hermes_cli/auth.py:307-315` (`has_usable_secret`) accepts any non-placeholder string >= 4 characters without querying any external service.

By adding:
```yaml
onboarding:
  run: false
model:
  provider: custom
  base_url: http://127.0.0.1:18899/v1
  default: test-model
  api_key: no-key-required
```
to the isolated `config.yaml` in `write_test_config()`, both `setup.status` and `setup.runtime_check` return success (`ok: True`, `provider_configured: True`). The webapp detects `runtimeReady: true` and completely skips mounting the ProviderPicker overlay in a fresh browser session, requiring no UI clicks or local storage tampering.

## Group chat hydration trials and known UI bug

The six seeded chats appear in the Sessions sidebar, and the seeded Bot Team is visible at `#/team`.
The server writes the Demo Product Room transcript through `merge_ui_meta` as a version-3 `hermes-bots-groups` projection on the default profile. However, in a fresh browser context, the four seeded room messages do NOT hydrate automatically.

Three trials were executed in isolated, clean Playwright browser contexts:
1. **First boot**: Navigated to launch URL, opened `#/team`, observed WebSocket frames and room transcript for 15 seconds. No `profiles.list` request with `include_sessions: false` was sent. The messages did not appear. (Screenshot: `/tmp/hermes-tauri-t2/group-chat-firstboot.png`).
2. **Backend restart**: With `#/team` open in the browser, the backend was stopped and restarted on the same port. The WebSocket reconnected, but no group-chat state pull was dispatched. Messages did not appear. (Screenshot: `/tmp/hermes-tauri-t2/group-chat-b-failed.png`).
3. **Page reload / 30s wait**: In a fresh context, after loading the launch link, `page.reload()` was called and the page was observed on `#/team` for 30 seconds. No hydration occurred. (Screenshot: `/tmp/hermes-tauri-t2/step-c-result.png`).

Because the room does not hydrate in a clean browser session without local storage injection, `/tmp/hermes-tauri-t2/group-chat-fresh.png` was not produced. Rendering of the room itself was visually confirmed using local storage injection in `/tmp/hermes-tauri-t2/group-chat-room.png`.

### Known UI bug: Group chat mirror hydration
- Plugins register at bundle load (`apps/desktop/src/app/contrib/controller.tsx:580`, calling `discoverBundledPlugins()`, and `apps/desktop/src/contrib/plugins.ts:57-65`, calling `plugin.register(...)`). This executes synchronously before any WebSocket connection or gateway instance exists.
- The registration hook (`apps/desktop/src/plugins/hermes-bots/plugin.tsx:438-445`, line 442) invokes `await pullGroupChatServerState().catch(() => false)`.
- `pullGroupChatServerState` (`apps/desktop/src/plugins/hermes-bots/group-chat.ts:1100-1121`) calls `groupChatRemoteSnapshot`, which invokes `groupChatSyncRequest` (`group-chat.ts:1079-1085`) to fetch `profiles.list` with `{ include_sessions: false }`.
- `groupChatSyncRequest` calls `host.request` (`apps/desktop/src/sdk/index.ts:1953-1958`). `host.request` checks `$gateway.get()`. Because the gateway is null before the socket connects, it immediately throws `Error('Hermes gateway unavailable')`.
- The `.catch(() => false)` in `plugin.tsx:442` swallows this error.
- Later, `host.state.gateway.listen(handleSessionsGatewayTransition)` (`apps/desktop/src/plugins/hermes-bots/plugin.tsx:459`) listens for gateway state changes, but `handleSessionsGatewayTransition` (`apps/desktop/src/plugins/hermes-bots/group-chat.ts:1426-1447`) does not initiate a pull for the active connection, and the initial gateway connection transition is replayed before `$gateway` is set.
- Consequently, on a fresh browser boot, the remote group chat snapshot is never pulled from the server mirror, leaving the group chat unhydrated until local storage is manually modified or populated.
- Furthermore, even when `pullGroupChatServerState()` is triggered after the gateway connects (e.g., in a warm profile or upon reconnect in `group-chat.ts:1444`), line 1060 calls `await host.profileRoutes()`. In `apps/desktop/src/sdk/index.ts:1642-1648`, `host.profileRoutes()` throws `Error('Hermes Desktop connection routing unavailable')` when `window.hermesDesktop?.getProfileRoutes` is missing, and that rejection aborts `pullGroupChatServerState`. Ruled out for the browser build (2026-10-15, headless Chromium on a disposable test Hermes, fresh profile and after reload): `typeof window.hermesDesktop?.getProfileRoutes` is `function` and `getProfileRoutes(['default'])` resolves, so `host.profileRoutes()` does not throw. The Tauri adapter (`src-tauri/src/adapter.js`) keeps the same bridge object and only overrides `openExternal` and `notify`, so the same holds there by code; its values were not read inside the webview. No `profiles.list` request with `include_sessions: false` reached the socket in any boot or reload, which fits the boot race above.

### Working UI creation path and diagnosis of T6p failure
In `proof.sh` step 05, creating a group chat through the UI works reliably:
1. Switch to the Bots tab at `(190, 48)` (not (168, 37), which misses the active pill).
2. Click the `+` menu trigger at `(259, 85)` and select "New group chat" at `(180, 142)`.
3. Select Bot 1 at `(550, 474)`.
4. In `create-dialog.tsx:1434-1465`, selecting Bot 1 mounts `<div className="flex flex-wrap gap-1">` (selection badge) and `<GroupRoleSelects>` above the bot list, dynamically shifting the bot list down by ~100px. T6p clicked the unshifted Bot 2 coordinate, clicking empty space, leaving only 1 bot selected. Because `canCreate = selected.length >= 2` (`create-dialog.tsx:1352`) and `<Button disabled={!canCreate}>` has `disabled:pointer-events-none` (`button.tsx:15`), T6p's click on Create Group was dropped.
5. Clicking Bot 2 at the shifted coordinate `(550, 590)` successfully selects 2 bots, activating `Create Group (2)` at `(785, 728)`.
6. Clicking `(785, 728)` mints a room ID, saves the room to local storage (`$groupChats`), and opens the group chat workspace.
7. Clicking the composer textarea at `(500, 710)`, clearing the draft, typing `"Demo: please summarize the launch checklist"`, and pressing `Return` submits the message, starting the live room round and displaying the bot's working state.

