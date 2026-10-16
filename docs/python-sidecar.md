# Bundled Python sidecar (Hermes + hermes_plus as a relocatable payload)

The Tauri shell ships its own Hermes backend so that "everything is a plugin" keeps working when
the user has no Hermes installed. A single relocatable payload holds a
[python-build-standalone](https://github.com/astral-sh/python-build-standalone) CPython, a
relocatable virtual environment with the pinned **stock Hermes** and the **`hermes_plus`** package
installed non-editable, the Desktop webapp renderer, and a small launcher. The payload is built by
[`scripts/build-python-payload.sh`](../scripts/build-python-payload.sh); nothing here touches
`src-tauri/`.

This document records the layout, how to build it, what was measured on the spike host, and a
concrete proposal for shipping it per platform. Anything not actually exercised on this host is
marked **not verified**.

## 1. What the payload runs

`hermes_plus serve` is the serving command. It starts a stock `hermes dashboard` child of its own
(readiness via the child's `/api/health`) and serves the Desktop UI itself from `--ui-dir`; its only
additions over the child are `/healthz` and `/api/plus/info`.

- `packages/hermes_plus/src/hermes_plus/cli.py` — `serve` subcommand.
- `packages/hermes_plus/src/hermes_plus/server/runtime.py` — spawns and manages the dashboard child,
  prints `Hermes Plus login: http://127.0.0.1:<port>/?token=…`.
- `packages/hermes_plus/src/hermes_plus/server/auth.py` — front-door token via
  `x-hermes-session-token` header/cookie or `?token=` on `/`; only `/healthz` and `/login` are
  unauthenticated.
- The renderer is the pin's `apps/desktop/dist-webapp`, copied into the payload and pointed to with
  `HERMES_PLUS_UI_DIR` (the launcher sets it).

The launcher exports `HERMES_PLUS_HERMES_BIN` (the bundled `hermes` console script) and
`HERMES_PLUS_UI_DIR` (the bundled `ui_dist`), then `exec`s
`venv/bin/python -m hermes_plus.cli serve "$@"`, so the process tree is
`hermes-sidecar → python (hermes_plus serve) → python (hermes dashboard child)`.

## 2. Payload layout

```
$HERMES_TAURI_PAYLOAD_DIR/            # default: $HOME/.cache/hermes-tauri/payload
  python/
    cpython-3.14.8-linux-x86_64-gnu/  # python-build-standalone CPython 3.14 (interpreter, stdlib)
    cpython-3.14-linux-x86_64-gnu     # uv's unversioned alias -> relative to the versioned dir
  hermes-payload/
    bin/hermes-sidecar                # POSIX launcher (see §1); all paths relative to itself
    venv/                             # relocatable uv venv: stock Hermes + hermes_plus (non-editable)
    ui_dist/                          # the pinned dist-webapp renderer, served via HERMES_PLUS_UI_DIR
    MANIFEST                          # python-version, hermes-plus, slim, built
```

`venv/bin/python` is a **relative** symlink into `../python/...`, and `python/cpython-3.14-…` is a
relative alias, so the whole tree can be moved. The build strips `__pycache__`, deletes
`*.dist-info/direct_url.json` (which names the source checkouts), and rewrites the generated
`_sysconfigdata*.py` so no file embeds the build path or the source trees. A `grep -rl` of file
*contents* plus an explicit symlink-target scan must both come back empty; the build fails if either
does not.

## 3. Building it

```sh
# From the worktree root. Idempotent: re-running refreshes the payload in place.
scripts/build-python-payload.sh [--slim] [--force] [--keep-pyc]

# Source discovery is overridable (both checkouts are otherwise found as siblings):
HERMES_PIN=<pinned-hermes-dir> \
HERMES_PLUS_SRC=<hermes_plus-package-dir> \
HERMES_TAURI_PAYLOAD_DIR=$HOME/.cache/hermes-tauri/payload \
  scripts/build-python-payload.sh
```

What the script does, in order: preflight free disk (stop below `HERMES_PAYLOAD_MIN_FREE_GB`, default
30G) → `uv python install --install-dir …/python 3.14` → `uv venv --relocatable` →
`uv pip install` the pin plus `hermes_plus` **non-editable** → copy the pin's `apps/desktop/dist-webapp`
into `ui_dist` → repoint the venv interpreter symlinks (and any absolute symlink inside the payload)
to relative targets → `--slim` pruning → write the launcher and `MANIFEST` → strip bytecode/installer
paths and self-check. Every install step runs under `flock /tmp/hermes-build.lock` and
`PYTHONDONTWRITEBYTECODE=1` is exported for the whole build so no `.pyc` bakes in a build path.

Why the pin is installed without extras: its `[web]`/`[mcp]` extras pin `starlette==1.3.1`, which
conflicts with `hermes_plus`'s `starlette>=0.37,<1`. Installing the pin core plus `hermes_plus`
resolves to `starlette 0.52.1` and `uvicorn 0.54.0` (70 packages). The consequence is that the
resolved `starlette` is **below** the `1.0.1` pin for CVE-2026-48710; that is a deliberate, recorded
trade-off, not an oversight.

`setup.py` in the pin refuses to build a wheel outside a Nix sandbox unless `HERMES_NIX_BUILD=1`, so
the script sets that for this deliberate packaging build only.

### Spawn command (`HERMES_TAURI_CMD`)

The shell resolves the payload under the resource directory and spawns the launcher directly (no
shell), so it controls the environment and the process group. Example JSON argv:

```json
["<payload>/hermes-payload/bin/hermes-sidecar", "--host", "127.0.0.1", "--port", "18800"]
```

Environment the shell should set: `HERMES_HOME` and `HERMES_RUNTIME_DIR` under the per-user app-data
directory (never inside the payload), plus `HERMES_DISABLE_LAZY_INSTALLS=1` and
`PYTHONDONTWRITEBYTECODE=1`. The launcher adds `HERMES_PLUS_HERMES_BIN` and `HERMES_PLUS_UI_DIR`.
Stop it by process group (the dashboard child runs in its own session, so a graceful `SIGTERM` to the
launcher's group is what makes the child exit; `SIGKILL` would leak it).

## 4. Measured numbers (this spike host)

Linux x86_64, ext4, shared uv cache, released under `flock`. CPython 3.14.8, Hermes
`v0.21.6+183.g8ac5c74`, `hermes-plus 0.1.0`. Full = default build; slim = `--slim`.

### Size on disk

| | python/ | hermes-payload/ | total (bytes) |
|---|---|---|---|
| full | 101,951,221 B (97.2 MiB) | 238,908,402 B (227.8 MiB) | **340,859,623 B (325.1 MiB)** |
| `--slim` | 87,708,786 B (83.6 MiB) | 238,908,402 B (227.8 MiB) | **326,617,188 B (311.5 MiB)** |

`du -sh` reports ~358 M (full) and ~337 M (slim) in blocks. Largest directories (full payload):
`venv/lib/python3.14/site-packages` 197 M, `python/…/cpython-3.14.8…` 107 M,
`hermes-payload/ui_dist` 54 M, `python/…/bin` 32 M, `python/…/lib` 66 M.

### Compressed size

Tarring `python/` + `hermes-payload/` together:

| codec | bytes | ratio | wall time |
|---|---|---|---|
| `xz -T4 -6` | **83,379,456 B (79.5 MiB)** | 0.245 | 1:03.75 |
| `zstd -T4 -19` | 87,716,289 B (83.7 MiB) | 0.257 | 4:15.70 |

### Cold start to `GET /api/health = 200`

Three fresh launches against an isolated `HOME`, polling the token-gated `/api/health`:

| run | ms |
|---|---|
| 1 | 16,219 |
| 2 | 19,768 |
| 3 | 17,514 |
| **median** | **17,514** |

Almost all of this is the `hermes dashboard` child booting; the payload's own Python import time is a
small fraction. This is the biggest cost of the bundled-backend approach and is worth a follow-up
(keep the child warm, or move the readiness probe earlier).

### Idle PSS after 30 s (two-process tree)

| process | PSS |
|---|---|
| `hermes_plus serve` (launcher) | 52.5 M |
| `hermes dashboard` child | 184.4 M |
| **total** | **236.9 M** (RSS 256.0 M) |

### What can be trimmed

Full-payload sizes of removable items. `__pycache__` is already stripped unconditionally (0 present),
so it saves nothing further here but must stay stripped because it embeds build paths.

| item | size | safe? |
|---|---|---|
| interpreter `share/terminfo` (curses DB) | 7.4 M | yes — headless server |
| interpreter `site-packages/pip` (+ `bin/pip*`) | 6.8 M | yes — build tooling |
| interpreter `include/` (C headers) | 2.6 M | yes — only for compiling |
| interpreter `ensurepip` | 1.8 M | yes |
| stdlib `idlelib` | 1.3 M | yes |
| interpreter `pydoc_data` | 616 K | yes |
| stdlib `tkinter`, `turtledemo`, `venv`, `lib2to3` | < 350 K each | yes |
| `.pyi` stubs / `dist-info` `RECORD` | 0.3 M / 0.6 M | yes |
| `pillow_heif.libs` / `nemo_relay` | 26 M each | **no** — imported by tools |
| `cryptography` | 15 M | **no** — auth/tokens |
| `pillow.libs` + `PIL` | 19.5 M | **no** — images |
| `anydoc`, `agent`, `openai`, `tools`, `gateway`, `plugins` | 5–8 M each | **no** — core features |

`--slim` applies the two biggest **safe** trims: (A) non-runtime interpreter files (`share/terminfo`,
`share/man`, `include/`, `config-*`, `pydoc_data`, and the stdlib `idlelib`/`tkinter`/`turtledemo`/
`ensurepip`/`lib2to3`/`venv`/`test`), and (B) build tooling (`pip`/`setuptools`/`wheel` in both the
venv and the interpreter, plus the `pip`/`idle`/`pydoc` console scripts). Measured saving:
340,859,623 − 326,617,188 = **14,242,435 B (13.6 MiB, 4.2 %)**. The dominant weight is third-party
dependencies that cannot be dropped without breaking features, so a slim build is a modest win only.

## 5. Shipping it per platform

### 5.1 `bundle.resources` vs `bundle.externalBin`

- **`externalBin`** ([docs](https://v2.tauri.app/develop/sidecar/)) embeds **executables**: each entry
  must exist with a `-$TARGET_TRIPLE` suffix (`…-x86_64-unknown-linux-gnu`, `…-aarch64-apple-darwin`,
  …), is placed next to the app binary, and is launched through the shell plugin's `sidecar()` API. It
  is designed for a single binary (e.g. a PyInstaller app), not a directory tree.
- **`bundle.resources`** ([docs](https://v2.tauri.app/develop/resources/)) copies arbitrary
  files/directories into `$RESOURCE`, preserving structure, and they are located at runtime with
  `app.path().resolve("…", BaseDirectory::Resource)`. This is the natural fit for our tree (interpreter
  + venv + `ui_dist`).

**Proposal:** ship the payload as **resources** and spawn the launcher from the resolved resource path
with `std::process::Command` (the shell already owns spawn/health/clean-shutdown in
`crates/hermes-sidecar`). `externalBin` is not used for the tree; if a single-file launcher is wanted
on Windows, a tiny per-triple `externalBin` stub can exec the resource tree, but that only buys a name
suffix, so prefer resources everywhere for one code path.

### 5.2 Where the payload lives at runtime

Two options, both work because the payload is relocatable:

- **Run in place under `$RESOURCE`.** No copy. Works for `deb` (`/usr/lib/<app>`), AppImage
  (squashfs mount), and macOS/Windows app bundles. The payload is read-only (writes are turned off
  with `PYTHONDONTWRITEBYTECODE=1` and a user-side `HERMES_HOME`), which suits hardened macOS and
  system-installed `deb`.
- **First-run unpack into the app-data directory.** Necessary where executing from `$RESOURCE` is
  blocked, and useful to get a stable path that survives AppImage's per-launch random mount point.
  Costs a second copy and needs an integrity check.

**Proposal:** run in place for `deb`, Windows installers, and macOS; on AppImage, run in place too —
the payload has **no absolute paths**, so the random `/tmp/.mount_*` path each launch is harmless. Keep
first-run-after-extract-and-run (`--appimage-extract-and-run`, no-FUSE hosts) as the one fallback.
Whether Tauri's resource copying preserves the launcher's executable bit on each platform is **not
verified**.

**Superseded:** running in place under `$RESOURCE` does not work. Tauri's resource copy dereferences
symlinks, and the payload's `venv/bin/python` symlink is what makes the venv find its stdlib, so the
copied payload dies with `Fatal Python error: Failed to import encodings module` (T16a, run A); with
the payload's two symlinks-to-directories staged, the bundler refuses to copy the tree at all. The
payload therefore ships as one archive and is unpacked on first run — see §5.8.

### 5.3 macOS signing

The payload contains many Mach-O files (the CPython binary, `lib-dynload/*.so`, and third-party
`.so`/`.dylib` such as `cryptography` and Pillow). Under the hardened runtime, **every** Mach-O in the
bundle must be signed with the app's identity (`codesign --force --options runtime --timestamp
--sign "$IDENTITY"`), and the whole bundle then notarized and stapled
([docs](https://v2.tauri.app/distribute/sign/macos/)). The `hermes-sidecar` shell script is not a
Mach-O and is left as-is. This must run as a post-build step, before Tauri signs and notarizes the
outer `.app`; otherwise the app is rejected on launch. Notarization is mandatory for a Developer ID
build. All of this is **not verified** on this Linux-only host; expect the nested-signing step to be
the main macOS work item.

### 5.4 Windows

Ship the python-build-standalone **windows-msvc** build (interpreter is `python.exe`; the venv's
`python.exe` is the entry point). The POSIX `#!/bin/sh` launcher does not run on Windows, so ship a
`hermes-sidecar.cmd` that derives its directory from `%~dp0` and `exec`s
`%~dp0..\venv\Scripts\python.exe -m hermes_plus.cli serve %*` (or a tiny Rust launcher exe). If the
launcher is delivered via `externalBin`, it needs the `-x86_64-pc-windows-msvc` suffix and the `.exe`
extension. Killing the tree needs a Windows job object (already the plan for the sidecar manager). All
**not verified** here.

### 5.5 Linux AppImage vs deb

Both bundle the payload as resources. `deb` installs under `/usr/lib/<app>` (root-owned, read-only,
updates replace it wholesale via the package manager). AppImage is a single file whose resources are
mounted read-only from squashfs each run. Neither can be written to, which is fine because the payload
never writes to itself. Prefer shipping both: `deb` for managed installs, AppImage for portable ones.
Tauri's own [AppImage](https://v2.tauri.app/distribute/appimage/) and
[Debian](https://v2.tauri.app/distribute/debian/) bundlers handle the packaging; only
`bundle.resources` changes in `tauri.conf.json`.

### 5.6 Update strategy

Two models:

- **Bundle and update with the app.** The payload version is pinned to the app version; the Tauri
  updater ([docs](https://v2.tauri.app/plugin/updater/)) replaces app + resources together, signed
  (`createUpdaterArtifacts`). One signed artifact, one version, no drift.
- **Separate payload channel.** Download and verify a payload archive independently into the app-data
  directory so Hermes/`hermes_plus` can update without an app release. Needs its own signed manifest,
  endpoint, and a compatibility contract between shell and payload.

**Proposal for v1:** bundle it and update with the app (simplest, fully signed). Add the separate
payload channel only if Hermes cadence demands it, and version the payload with the `MANIFEST`
(`python-version`, `hermes-plus`, `built`) plus a payload semver so the shell can refuse an
incompatible pair.

### 5.7 How user-installed Python plugins coexist

The payload lives inside the app (resource dir) and is replaced on every update, so **nothing the user
installs may go there**. User state — Hermes home, config, sessions, and user Python plugins — belongs
in the per-user app-data directory (`$XDG_DATA_HOME`/`$XDG_CONFIG_HOME`, `~/Library/Application
Support`, `%APPDATA%`), which the shell points at via `HERMES_HOME`. The backend loads plugins from
that home exactly as it does today, so app/payload updates replace only the immutable payload and
never wipe user plugins (see also the plan's "Python plugins are unaffected"). The shell must never
write into `$RESOURCE`.

### 5.8 Archive + first-run unpack (T16b)

The payload ships as **one archive plus a manifest** in the resource dir, and the shell unpacks it into
the app-data dir on first run:

```json
"bundle": { "resources": { "payload/hermes-payload.tar.zst": "hermes-payload.tar.zst",
                           "payload/payload-manifest.json": "payload-manifest.json" } }
```

`scripts/pack-payload.sh` writes both into `src-tauri/payload/` (gitignored). The tar stream keeps the
payload's relative symlinks, which is the whole point: a resource copy dereferences `venv/bin/python`
and the venv then dies with `Fatal Python error: Failed to import encodings module` (T16a, run A). The
archive is deterministic (names sorted, every mtime the epoch, numeric owner 0, no build path in any
name — packing the same tree twice gives byte-identical output, on the host and in the builder), and
the script refuses to pack a tree whose `*.pyc` files embed the build path. A payload freshly built by
`scripts/build-python-payload.sh` passes that gate (its build exports `PYTHONDONTWRITEBYTECODE=1` and
its last step strips every `__pycache__`, so it holds no `.pyc` at all): the build-path `.pyc` this
host once carried were written *at runtime*, by boots of the payload in place before the shell's
`PYTHONDONTWRITEBYTECODE` spawn env existed (T10's measurement runs; T21 confirmed that one un-scrubbed
import writes `.pyc` embedding the run path, while the shell's own env writes none). Never pack a tree
that ran in place — rebuild it first, or pass `--allow-build-paths` for a local experiment only.
`payload-manifest.json` is `{ version, sha256, archive_bytes, unpacked_bytes, files }`, where `version`
is the payload's own `hermes-payload/MANIFEST` `built` field (`2026-10-15T07-23-19Z`), sanitised into
one directory name.

`hermes-shell-config::ensure_unpacked(archive, manifest, data_dir)` installs it, and only ever writes
inside `<data_dir>/payload`:

```
<data_dir>/payload/.lock             exclusive install lock (a second shell blocks, then sees .ready)
<data_dir>/payload/<version>/        the unpacked payload (hermes-payload/ + python/)
<data_dir>/payload/<version>/.ready  the archive sha256 it was unpacked from
<data_dir>/hermes                    HERMES_HOME         (T15 bundled_env — never touched here)
<data_dir>/runtime                   HERMES_RUNTIME_DIR  (T15 bundled_env — never touched here)
```

It returns at once when `.ready` names the manifest's sha256. Otherwise it verifies the archive hash
*before* writing anything, unpacks into `<version>.tmp-<pid>` refusing absolute paths, `..`
components, hard links and symlinks that leave the destination (every symlink in the payload itself
stays inside), keeps the exec bits, renames the staging dir into place, writes `.ready` atomically, and
only then prunes stale staging dirs and older versions. The returned `Bundled` is what T15's
`resolve_with_bundled` takes.

Measured on the real 341 MB payload (T16b, inside `hermes-tauri-builder:26.04`):

| | |
|---|---|
| archive | 88,407,662 B (84.3 MiB), zstd level 19 with 4 workers, 52-53 s to pack |
| archive entries | 13,708 (11,762 files, 1,053 symlinks, 893 directories) |
| cold unpack | 13.7 s (327.0 MiB written into the app-data dir) |
| second call (`.ready`) | < 1 ms |
| boot to `/healthz` = 200 | 14.4 s (T15 measured 16.1 s, T10 16-20 s) |

T16's final archive, packed from the refreshed payload (see §5.9), is bigger because it is a newer
`hermes_plus`, and it is the clean one T21 packs without `--allow-build-paths`: 90,932,314 B (86.7 MiB),
sha256 `2b4ca0ae…`, version `2026-10-15T23-00-58Z`, 14,117 entries (12,092 files, 1,053 symlinks, 972
directories), 347,162,888 B unpacked, and no `*.pyc` at all (the pack gate finds 0 of 0).

**Use `ready_timeout >= 90 s` for the first run.** It pays the unpack (14 s here) *plus* the cold start
(14-53 s measured, 46-47 s while the machine was busy), and the sidecar's 60 s default does not cover
both.

**T16 wired it** — see §5.9 for the shell side (resource/app-data dirs, the boot page phase, the build
line, the measured sizes, and the environment the payload must *not* inherit).

### 5.9 Shell wiring (T16)

The shell installs and spawns the payload itself, and never on the thread that draws the window:

| step | where |
|---|---|
| find the archive + manifest in the resource dir | `src-tauri/src/payload.rs:41` (`locate`), called from `src-tauri/src/main.rs:69` |
| hand them to the boot manager | `BootManager::new(config_dir, payload, log)`, `src-tauri/src/sidecar.rs:189` |
| start the first attempt | `BootManager::prepare`, `sidecar.rs:222`, from setup (`main.rs:174`) |
| unpack on the attempt thread | `sidecar.rs:315` (`resolve`) → `payload::unpack` (`payload.rs:58`) → `ensure_unpacked` |
| resolve against the unpacked payload | `config::load(config_dir, Some(&bundled))`, `src-tauri/src/config.rs:43` |
| ready timeout | `config.rs:86` (`with_bundled_timeout`) raises it to 90 s (`BUNDLED_READY_TIMEOUT`, `config.rs:25`) only when the resolved command lives under the payload dir (`spawns_the_payload`, `config.rs:101`) |
| boot page | the `preparing` phase (`boot/boot.js:23`) shows "Preparing Hermes (first run)…"; an install error publishes the error state with Retry (`sidecar.rs:315`, never a panic) |

`locate` needs **both** files, so a build without the payload resources (the plain `cargo tauri build`)
falls back silently to `HERMES_TAURI_URL`/`HERMES_TAURI_CMD`/`config.toml`/PATH: one binary serves both
bundles.

Release procedure: build the payload into its own directory, pack it WITHOUT `--allow-build-paths`
(the pack gate then also proves the tree carries no build path), and bundle:

```sh
HERMES_TAURI_PAYLOAD_DIR=$HOME/.cache/hermes-tauri/payload-clean \
  scripts/build-python-payload.sh
scripts/pack-payload.sh --payload "$HOME/.cache/hermes-tauri/payload-clean" --out src-tauri/payload
scripts/builder.sh bash -c 'cd src-tauri && cargo tauri build --bundles deb,appimage --config tauri.payload.conf.json'
```

A fresh build passes the pack gate with no flag (§5.8); `--allow-build-paths` is for local
experiments only, never a shipped artefact. It was needed while the payload had been booted in place,
because those boots wrote `*.pyc` embedding the build path; a payload built as above carries **no**
`*.pyc` at all, so `pack-payload.sh` passes its own check with no flag (a flag-free repack reproduces
the identical sha256).

Measured here (T16 final, `hermes-tauri-builder:26.04`, payload archive 90,932,314 B / 86.7 MiB):

| bundle | deb | AppImage |
|---|---|---|
| plain (no payload resources) | 8,961,586 B (8.55 MiB) | 91,634,168 B (87.39 MiB) |
| payload (`--config tauri.payload.conf.json`) | 99,832,556 B (95.21 MiB) | 182,503,928 B (174.05 MiB) |
| delta | 90,870,970 B (86.66 MiB) | 90,869,760 B (86.66 MiB) |

The delta is the archive, which both formats store compressed and neither re-compresses much. The
347,162,888 B (331 MiB) unpacked copy lands in the app-data dir on first run and is not part of either
bundle.

**A spawned payload must not inherit the AppImage's interpreter environment.** The AppImage's `AppRun`
exports `PYTHONHOME=$APPDIR/usr` and `PYTHONPATH=$APPDIR/usr/share/pyshared` for the GTK pieces it
bundles. A payload launcher that inherits them dies before its first line of Python (`Fatal Python
error: Failed to import encodings module`, the same message as T16a's dereferenced copy, from a
different cause), so `crates/hermes-sidecar/src/connect.rs` (`SCRUBBED_ENV`, `scrub_env`) removes those
variables from every child the shell spawns. `LD_LIBRARY_PATH` is deliberately left alone: the AppDir's
libraries do not disturb the payload (measured), and they can only help on a host that lacks one.

The scrub is the same for every spawned backend, whatever named it: the bundled payload, a
`HERMES_TAURI_CMD` command and a `config.toml` `command` all go through that one `spawn()`. It is a
`env_remove` applied *after* the configured env, and `env_remove` wins, so those eight names cannot be
handed to a backend through `SidecarConfig.env` either (the review's suggested escape hatch does not
exist): a launcher that needs one has to set it itself, in its own process. Every other variable the
config names does reach the backend, which is what the fixture asserts end to end
(`crates/hermes-sidecar/tests/spawn.rs`, `spawned_backend_does_not_inherit_the_shells_python_env`:
`PYTHONHOME`/`PYTHONPATH` from the config arrive unset, `HERMES_HOME` from the same config arrives).

**Password mode.** With `auth = "password"` the front door serves a token-less `/login`, so the shell's
"the launch page consumed its credential" check (`sidecar.rs:670`, `launch_consumed`) is true at once
and a deep link arriving during login can preempt the login page. The payload defaults to `token`
(`hermes_plus/server/settings.py`), so this affects only an explicit `auth = "password"`.

## 6. Status of claims

- **Verified on this host:** build (full + `--slim`), relocation by `cp -a` and run from the moved
  copy (0 content hits, 0 bad symlinks), `/api/health` 200, `/api/plus/info`, UI index served, graceful
  stop with no leftovers, all sizes/timings above.
- **Not verified:** every macOS item (nested Mach-O signing, notarization), every Windows item
  (windows-msvc payload, `.cmd`/exe launcher, job objects), executable-bit preservation when Tauri
  copies resources, and AppImage/deb end-to-end packaging.
