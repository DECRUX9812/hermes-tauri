# Disposable test Hermes

`scripts/test-hermes.sh start` builds no dependencies and starts the pinned Desktop webapp against a Hermes instance isolated under `/tmp/hermes-tauri-test`. It chooses a free port in `18800–18899`, seeds neutral sessions and Bots demo content, waits for `/api/health`, and prints only the origin. The server uses separate `HOME`, `HERMES_HOME`, and `HERMES_RUNTIME_DIR` paths, and the isolated config disables onboarding. No model provider or API key is required.

Use `scripts/test-hermes.sh url` to retrieve the private launch link, `status` to check health, `cmd` to print the shell-escaped `HERMES_TAURI_CMD` assignment, and `stop` to terminate the server process group. `reset` stops the managed server and removes only `/tmp/hermes-tauri-test`. The launch link is stored with mode `0600`; do not paste it into logs or reports.

The script uses the Python environment selected by the existing Hermes launcher. Set `HERMES_TAURI_PYTHON` to an existing Hermes-compatible interpreter if automatic discovery does not apply. It does not install or alter Python dependencies.

The isolated config disables the backend onboarding flow. The browser may still show its first-run provider picker because that dismissal is stored in browser-local storage; choose “I’ll choose a provider later.” No provider key is needed.

The six seeded chats appear in the Sessions sidebar. The seeded Bot Team is available at `#/team`. The seed also writes a four-message `Demo Product Room` projection to the default profile's `ui_meta`, which `profiles.list` returns. In this pinned build, a clean browser did not hydrate that transcript into its local Bots store. The captured UI screenshot used the same neutral messages in the isolated browser's local group-chat storage; see `.oc/reports/T2.md` for the verification details.

The plugin rail is served by the pinned webapp and can be checked with `/api/desktop-plugins/list`. This isolated setup returned zero desktop-plugin entries; the built-in app navigation still rendered.
