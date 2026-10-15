# WebKit compatibility audit

Scope: the read-only Hermes desktop pin at `_hermes-pin/apps/desktop` (`src/` and the built `dist-webapp/`). The pin was not modified. Runtime routes were tested against the isolated test server on the 18800 port range.

## Engine versions

WebKitGTK and Safari do not share an exact public version map. The Safari column below is an engine-era approximation from release dates, not a claim that the builds are identical.

| Environment | Exact version observed | Approximate Safari era | Evidence |
|---|---:|---|---|
| Host runtime library | WebKitGTK 4.1 **2.52.6-0ubuntu0.26.04.1** | Safari 26.4–26.6 era | `ldconfig -p` exposes `libwebkit2gtk-4.1.so.0`; `dpkg-query` reports 2.52.6. |
| Linux builder image `hermes-tauri-builder:26.04` | WebKitGTK 4.1 **2.52.6-0ubuntu0.26.04.1** | Safari 26.4–26.6 era | `docker run --rm --memory 2g hermes-tauri-builder:26.04 dpkg -l` reports the runtime and dev packages. |
| `ubuntu-22.04` CI apt floor, current package candidate | WebKitGTK **2.50.4-0ubuntu0.22.04.1** | Safari 26.0 era | Ubuntu Jammy package record for `libwebkit2gtk-4.0-37` / `webkit2gtk`. The task note said “2.4x”; that is historical (for example 2.44.3 exists), while the current Jammy security candidate is 2.50.4. |
| Playwright browser downloaded for comparison | WebKit **26.5**, Playwright build `webkit-2336` | Safari 26.5 era | Downloaded successfully, but the host lacks `libevent-2.1-7t64`, `libavif16`, and `libmanette-0.2-0`, so it did not launch. |
| Runtime smoke fallback | Host WebKitGTK **2.52.6** | Safari 26.4–26.6 era | Real PyGObject `WebKit2.WebView` under Xvfb. This is the same engine family the Linux shell links. |
| Chromium control | Chrome for Testing **151.0.7922.34** | not applicable | Same smoke script and routes. |

Host feature probe under WebKitGTK 2.52.6: `Array.prototype.findLast`, `structuredClone`, `Intl.Segmenter`, `:has()`, `color-mix()`, `field-sizing`, `scrollbar-gutter`, WebGL, and WebGL2 are present. `requestIdleCallback` and `cancelIdleCallback` are absent.

## Static compatibility findings

Severity is assessed against two audiences: the current 2.50.4/2.52.x target, and older distro WebKitGTK installs that an AppImage may encounter. “Breaks” means code can throw or a feature cannot work; “degrades” means a visible feature falls back or is delayed; “cosmetic” means styling only.

| Feature | Evidence (paths are relative to `apps/desktop`) | Minimum / behavior | Fallback in code | Severity |
|---|---|---|---|---|
| `backdrop-filter` | Paired prefixed declarations: `src/styles.css:1558-1559`, `src/lib/tour/app-tour.css:126-127`, `src/app/botroom/engine/styles.ts:65`; unprefixed-only examples: `src/styles.css:800`, `src/lib/tour/app-tour.css:19`; built CSS: `dist-webapp/assets/index-CyR5mpuM.css:2` | Prefixed WebKit support is longstanding; unprefixed support is Safari 18-era. | Most critical surfaces include `-webkit-backdrop-filter`; several overlay declarations do not. | **Cosmetic/degrades** on old engines; no issue observed on 2.52.6. |
| CSS `:has()` | `src/styles.css:818-824`, `src/app/chat/composer/status-stack/drawer.css:45`, `src/plugins/hermes-bots/group-chat-view.room-theme-anchors.test.tsx:122-130` documents selectors; built CSS at `dist-webapp/assets/index-CyR5mpuM.css:2` | Safari 15.4 / WebKitGTK 2.36-era and later. | No selector fallback. | **Degrades** below the floor; current target is fine. |
| Container queries | `src/styles.css:2130-2133`, `src/app/right-sidebar/review/scope-row.tsx:31`, `src/app/settings/primitives.tsx:182,285`, `src/plugins/hermes-bots/group-chat-view.tsx:1400` | Safari 16.0. | No non-container-query layout fallback. | **Degrades** on old engines; current target is fine. |
| `@layer` | `src/styles.css:204`, `src/styles.css:1681`; built CSS at `dist-webapp/assets/index-CyR5mpuM.css:2` | Safari 15.4. | No unlayered duplicate for layered rules. | **Breaks styling** below the floor; current target is fine. |
| `color-mix()` | Token core: `src/styles.css:176-218,304-327`; design tokens: `src/design/tokens.css:85-90`; inline use: `src/plugins/kanban/ui.tsx:211`; built CSS at `dist-webapp/assets/index-CyR5mpuM.css:2` | Safari 16.2. | Tailwind emits a few `@supports` fallbacks, but the app’s raw token chain has no general sRGB duplicate. | **Breaks theme styling** below the floor; current target is fine. |
| `oklch()` and relative color | `src/styles.css:471`, `src/app/shell/butterbar.css:70`; generated palette at `dist-webapp/assets/index-CyR5mpuM.css:2` | Absolute `oklch()` is Safari 15.4; relative color syntax is about Safari 16.4. | No general fallback. | **Degrades** below the floor; current target is fine. |
| `field-sizing` | `src/components/ui/search-field.tsx:104-114`, `src/app/right-sidebar/review/ship-bar.tsx:97-101`, `src/plugins/hermes-bots/group-chat-parts.tsx:293-295` | Safari 18.4-era. | Elements keep explicit height/flex rules, so the effect is sizing polish rather than a functional gate. | **Degrades** on old engines; host 2.52.6 reports support. |
| `text-wrap: balance` | No app source hit; the generated stylesheet was scanned at `dist-webapp/assets/index-CyR5mpuM.css:2`. | Safari 17-era where used. | Not applicable. | **None** today. |
| `scrollbar-gutter` and scrollbar styling | WebKit pseudo-elements: `src/styles.css:1693-1717`; standard fallback gate: `src/styles.css:1802-1837`; gutter uses: `src/app/chat/sidebar/index.tsx:224-227`, `src/app/master-detail.tsx:158,177`, `src/app/artifacts/index.tsx:414` | `::-webkit-scrollbar` is WebKit-native; `scrollbar-gutter` is Safari 18.2-era. | The WebKit scrollbar path already exists; missing gutter only allows minor layout shift. | **Cosmetic**; host 2.52.6 reports gutter support. |
| View transitions | No app API hit. The built hit is syntax-highlighter grammar data, not an app call: `dist-webapp/assets/shiki-CsbwoIFU.js:201`. | Safari 18-era where used. | Not applicable. | **None** today. |
| Native `popover` | `src/lib/preview-act/watch-in-page.ts:305-308,943-950`; built chunk `dist-webapp/assets/preview-act-B2gXF_au.js:9` | Safari 17.0. | Calls are caught and the existing z-index fallback remains. | **Degrades** below Safari 17; not used by the main rail. |
| Native `<dialog>` | No native app use; the UI component is a Radix wrapper at `src/components/ui/dialog.tsx:1-12`. | Safari 15.4 where native dialog is used. | Radix owns focus/portal behavior. | **None** today. |
| `structuredClone()` | `src/lib/katex-memo.ts:193-200`, `src/components/pane-shell/tree/presets.ts:219-226`, `src/app/settings/helpers.ts:233-235`, `src/app/settings/billing/simulated-api.ts:30`; built example `dist-webapp/assets/presets-B0ktI95X.js:1` | Safari 15.4. | No local fallback. | **Breaks** affected panes below the floor; host 2.52.6 has it. |
| `Array.prototype.findLast()` | `src/lib/runtime-turn-merge.ts:254,329,362`, `src/lib/inflight-turn-merge.ts:139,184`, `src/app/session/hooks/use-message-stream/index.ts:704,1081,1183`; built example `dist-webapp/assets/session-CSs37GpI.js:2` | Safari 15.4. | No local fallback. | **Breaks** session/stream paths below the floor; host 2.52.6 has it. |
| `Array.prototype.toSorted()` | No native app call. The built `toSorted` at `dist-webapp/assets/dropdown-menu-CVDDBWzz.js:1` is a method on a custom collection class. | Safari 16-era where native use exists. | Not applicable. | **None** today. |
| `Intl.Segmenter` | Module-scope construction: `src/lib/profile-short-label.ts:1`, `src/app/cron/job-state.ts:22-30`; built example `dist-webapp/assets/job-state-oEJkaw0Z.js:1` | Safari 14.1. | No local fallback. | **Breaks** importing those modules on very old engines; current target is fine. |
| `requestIdleCallback()` | Existing fallbacks: `src/components/idle-mount.tsx:11-24`, `src/app/route-chunks.ts:22-30`, `src/components/assistant-ui/embeds/mermaid-render-cache.ts:60-64` | Not exposed by the tested WebKitGTK 2.52.6, even though modern JavaScript/CSS probes are otherwise present. | Yes: eager mount or `setTimeout`. | **Degrades scheduling** on current WebKitGTK; a tiny shell shim is recommended, but not committed here because `src-tauri` is owned by another task and this branch has no `src-tauri/Cargo.toml`. |
| `ResizeObserver` loop behavior | Guarded/shared hook: `src/hooks/use-resize-observer.ts:35-38,81-87`; direct observers include `src/components/assistant-ui/thread/list.tsx:563,787` | Safari 13.1. | Shared hook calls with empty entries when unsupported; no callback that resizes its observed target in a loop was found. | **Degrades** on very old engines; no runtime errors observed. |
| Clipboard | Policy wrapper: `src/lib/clipboard.ts:6-37`; browser reads/writes: `src/lib/browser-clipboard.ts:13-44`; image write: `src/lib/svg-image.ts:146-148`; user-triggered paste path: `src/app/context-menu/app-context-menu.tsx:110-118` | Safari 13.1+, secure context, and user activation/permission rules. | Electron text writes have IPC; browser-host mode intentionally throws when unavailable. | **Degrades** when WebKit denies activation or permission. |
| WebGPU / WebGL2 | Three.js surfaces: `src/app/pane3d/pane-app.tsx:117-127`, `src/app/botroom/engine/mascot3d.ts:160-167`, `src/plugins/agentcraft/scene.ts:103-115`; terminal fallback: `src/app/right-sidebar/terminal/use-terminal-session.ts:436-458` | WebGL2 is Safari 15-era; no WebGPU app call found. | Terminal falls back to DOM rendering; Three.js scenes do not have a non-WebGL scene. | **Breaks 3D surfaces** if WebGL is unavailable; runtime probe has WebGL/WebGL2 and studio renders. |
| `getUserMedia` / `MediaRecorder` | Capability checks and MIME selection: `src/app/chat/composer/hooks/use-mic-recorder.ts:83-97,288-328`; wake capture checks: `src/lib/wake-client-capture.ts:98-119` | `getUserMedia` is Safari 11-era; `MediaRecorder` is Safari 14.1-era. WebKitGTK recording still depends on GStreamer and permission policy. | Yes: capability checks, `isTypeSupported`, constructor error handling, and user-visible failure. | **Degrades** where the host has no microphone pipeline or permission delegate. |
| Notifications | Browser bridge guard: `src/lib/browser-desktop-bridge.ts:99-107`; room preference catches denied/unsupported cases: `src/plugins/hermes-bots/group-room-prefs.ts:126-156` | Permission-gated in Safari/WebKitGTK. | Bridge returns false when unavailable; shell should overlay native notifications. | **Degrades** without shell-native notification support. |
| File System Access / `showSaveFilePicker` | No source call. Excalidraw’s generated optional path is `dist-webapp/assets/file-save-745eba88-Bwdfz6OZ.js:1`; caller feature-checks at `dist-webapp/assets/chunk-K2UTITRG-CfrLW4Kv.js:8`; anchor fallback at `dist-webapp/assets/file-save-3189631c-CO9S4HFW.js:1`. Source entry: `src/app/chat/right-rail/canvas-surface.tsx:1-3` | Not available in Safari/WebKitGTK. | Existing legacy anchor download path. | **None** for this path. |
| Plugin loader `import(blob:)` | Blob URL and dynamic import: `src/contrib/runtime-loader.ts:398-419`; failure is surfaced: `src/contrib/runtime-loader.ts:528-542` | Dynamic import is Safari 11.1-era; `blob:` module acceptance also depends on CSP. | No alternate evaluator, but failures become plugin-row errors. | **Breaks runtime plugins** if CSP or the engine rejects blob modules; no error appeared in the current smoke routes. |
| Ticketed WebSocket | Ticket mint and URL construction: `src/lib/browser-transport.ts:276-296`; reconnect remints: `src/app/gateway/hooks/use-gateway-request.ts:121-123` | WebSocket itself is Safari 6-era. Tickets avoid custom headers. | Reconnect remints tickets. | **None observed**; seeded HTTP/WS-backed views rendered in both engines. |
| `BroadcastChannel` | Guarded sync: `src/store/session-sync.ts:8`; guarded preview handoff: `src/lib/preview-annotate/handoff.ts:353-357`; unguarded private-window handoff: `src/lib/browser-window.ts:18-20` | Safari 15.4. | Partial. `browser-window.ts` should feature-check before constructing. | **Breaks that handoff path** below the floor. |
| `SharedWorker` | No app worker creation. The built hit is a denied constructor name in Markdown deserialization: `dist-webapp/assets/vendor-md-DAe5ygxp.js:1`. | Safari 16-era where used. | Not applicable. | **None** today. |
| Service workers and custom schemes | No service-worker registration hit. Electron owns custom schemes at `electron/main.ts:1882-1895,1898-1945`; browser media uses HTTP stream URLs at `src/lib/browser-files.ts:75-83`. | Service workers do not run on arbitrary custom schemes; Tauri’s remote origin is normal loopback HTTP. | Browser-host path already avoids custom media schemes. | **None** if the shell keeps the remote `http://127.0.0.1` origin. |
| IndexedDB | No source or generated app hit; scanned generated entry point `dist-webapp/assets/index-CefJxBEK.js:2-3`. | Safari 10-era where used. | Not applicable. | **None** today. |
| Date inputs | No `type="date"` app hit; scanned source input primitive at `src/components/ui/search-field.tsx:99-119` and built entry `dist-webapp/assets/index-CefJxBEK.js:2-3`. | Safari date controls have different styling where used. | Not applicable. | **None** today. |
| `:focus-visible` | Input modality detection: `src/lib/input-modality.ts:3-9`; component style: `src/components/ui/button.tsx:21`; stylesheet example: `src/styles.css:1180` | Safari 15.4. | No broad `:focus` duplicate. | **Accessibility degradation** below the floor. |
| CSS `zoom` | Browser zoom bridge sets the property at `src/lib/browser-zoom.ts:16-19`; bridge exposure at `src/lib/browser-desktop-bridge.ts:142`. | Longstanding WebKit support. | Not needed. | **None** today. |

## Runtime smoke result

Command shape (the token was supplied through `WEBKIT_SMOKE_URL` and redacted from output):

```sh
PLAYWRIGHT_MODULE=<playwright-index.mjs> \
WEBKIT_SMOKE_OUTPUT=/tmp/hermes-tauri-t8 \
WEBKIT_SMOKE_URL=<private-launch-link> \
node scripts/webkit-smoke.mjs
```

The script first tried Playwright WebKit. That browser could not launch because the host lacks the three packages listed above. It then used the real host WebKitGTK 2.52.6 fallback in `scripts/webkitgtk-smoke.py` under Xvfb. Chromium used Playwright’s Chrome for Testing 151.0.7922.34.

| Route | WebKitGTK console/page errors | Chromium console/page errors | Visual result |
|---|---:|---:|---|
| boot | 0 / 0 | 0 / 0 | Launch token consumed; no URL/token appeared in output. |
| `#/` | 0 / 0 | 0 / 0 | Both engines show the initialized session lobby, seeded bots/sessions, composer, and recent-task cards. |
| `#/team` | 0 / 0 | 0 / 0 | Both show the seeded team page, member cards, goals, learning, and activity sections. |
| `#/kanban` | 0 / 0 | 0 / 0 | Both show the empty board and guidance banner. |
| `#/studio` | 0 / 0 | 0 / 0 | Chromium rendered the 3D scene after the default short wait. WebKitGTK still showed the boot message at 1.5 s, then rendered the 3D scene by 6 s. Probe: WebGL true, WebGL2 true, 250 resources, 0 incomplete images. |
| `#/settings` | 0 / 0 | 0 / 0 | Both show the settings dialog and main-model page. |

Runtime diff: no console or page-error diff. The only observed difference is slower studio startup under software-rendering Xvfb WebKitGTK; it completed and rendered successfully. Screenshots are `/tmp/hermes-tauri-t8/{webkit,chromium}-{home,team,kanban,studio,settings}.png`.

## Recommended fixes

### Shell-side (Tauri)

1. Keep `WEBKIT_DISABLE_DMABUF_RENDERER=1` as the Linux default unless the user overrides it. This avoids the known blank/black DMABUF path on NVIDIA and Xvfb.
2. Wire a tiny `requestIdleCallback`/`cancelIdleCallback` init-script shim before app code. The shim can be a guarded `setTimeout` approximation with a monotonic `timeRemaining()`. It is not committed on this branch because creating `src-tauri/src/polyfills.js` before the shell scaffold exists would make the workspace’s `src-tauri*` member fail to load; the UI already takes its fallback paths.
3. Keep WebGL enabled in WebKitGTK settings. The studio route needs it and rendered successfully on the host; add telemetry or a user-visible shell error only for context-creation failure.
4. Handle WebKitGTK permission decisions explicitly for microphone, notifications, and clipboard-sensitive actions. The UI already feature-checks these APIs, but the shell needs the native delegate/policy so denials are intentional and recoverable.
5. Keep the remote capability scoped to the exact loopback backend origin and keep the backend CSP compatible with `script-src 'self' blob:` for the runtime plugin loader.
6. For CI, install Playwright WebKit with its system dependencies (`npx playwright install --with-deps webkit`) or run the host WebKitGTK fallback in the builder image. The local Playwright browser download alone was not sufficient.
7. Keep Linux WebKit smoke tests on both the current Jammy candidate and the builder image; AppImage users can bring an older host WebKitGTK than CI.

### UI-side (hermes-next / hermes-desktop-plus)

1. Add sRGB fallbacks before raw `color-mix()`/`oklch()` token declarations if the shell will support WebKitGTK older than the Safari 16.2/16.4-era feature floor.
2. Add `-webkit-backdrop-filter` to the remaining unprefixed-only blur declarations, or accept a documented no-blur fallback on Safari 17 and older.
3. Add `:focus` fallbacks where focus styling is essential for keyboard operation on pre-15.4 WebKit.
4. If older distro WebKitGTK remains supported, guard or transpile direct `structuredClone()`, `Array.prototype.findLast()`, and module-scope `Intl.Segmenter` calls. Current CI and this host do not need those shims.
5. Feature-check the unguarded `BroadcastChannel` construction in `src/lib/browser-window.ts`.
6. Catch `Notification.requestPermission()` rejection in the browser bridge and keep returning `false` rather than surfacing an unhandled rejection.
7. Keep the existing clipboard policy wrapper and surface a manual-copy path when WebKit denies clipboard access outside a user gesture.
8. Give the studio route a visible timeout/error if `createStudio()` remains pending, and consider reducing startup work or showing progress. This was a delay, not a failure, on WebKitGTK 2.52.6.
9. Keep the existing `MediaRecorder.isTypeSupported` list and include an MP4/AAC candidate for WebKit; do not assume WebM recording exists.
10. Add the reusable smoke script to the normal UI compatibility workflow so future CSS/API changes are checked against WebKit before release.
