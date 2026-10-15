# demo-settings fixture plugin

Neutral fixture used by T18 to prove that Hermes' own Settings > Plugins page works unchanged in
the Tauri (WebKitGTK) shell. It is never installed by default: `scripts/test-hermes.sh` copies it
into the disposable test Hermes only when `HERMES_TAURI_DEMO_SETTINGS_PLUGIN=1`.

`plugin.yaml` declares one key per settings field type the Desktop form renders (`str`,
`str` + `choices`, `int`, `float`, `bool`, `list`, `dict`, and `secret` with `env:`), each with a
`label`/`description`/`default`, and `api_url` marked `required`. `__init__.py` registers nothing
and only reads `mode` back through `ctx.get_config` so the saved value is visible in the log.
