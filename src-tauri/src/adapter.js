(() => {
  // Runs on every page the webview loads. In SPAWN mode the backend origin
  // (port 0) is unknown when the window is built, so the adapter cannot be
  // scoped by origin here. The real IPC boundary is the runtime capability,
  // which is granted to the exact backend origin after connect resolves, and
  // the navigation guard keeps every other origin out of this webview. The
  // adapter therefore only overlays on http(s) pages, never on the boot page.
  if (location.protocol !== 'http:' && location.protocol !== 'https:') return;

  const tauri = window.__TAURI__;
  if (!tauri?.core?.invoke) return;
  const markNativeHost = () => {
    if (document.documentElement) {
      document.documentElement.dataset.hermesNativeHost = 'tauri';
      return;
    }
    document.addEventListener('DOMContentLoaded', markNativeHost, { once: true });
  };
  markNativeHost();

  const nativeOpenExternal = async url => {
    if (typeof url !== 'string') return;
    let parsed;
    try {
      parsed = new URL(url);
    } catch {
      return;
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return;
    await tauri.core.invoke('native_open_external', { url: parsed.href });
  };
  const nativeNotify = async payload => {
    try {
      return await tauri.core.invoke('native_notify', {
        payload: payload && typeof payload === 'object' ? payload : {}
      });
    } catch {
      return false;
    }
  };
  const nativeSelectPaths = async options => {
    try {
      return await tauri.core.invoke('native_select_paths', {
        options: options && typeof options === 'object' ? options : {}
      });
    } catch {
      return [];
    }
  };

  // WebKitGTK has no default reload accelerator (Electron reloads on F5 and
  // Ctrl/Cmd+R). Reload pages of the backend UI only: the session token the
  // bootstrap consumed lives in sessionStorage (`hermes.webapp.session.*`,
  // written by the UI on the backend origin), so its presence identifies a
  // page where reloading keeps an authenticated session. Never reload a
  // foreign page, and never during IME composition or while the key auto-
  // repeats (review F5, F6).
  const canReload = () => {
    if (location.protocol !== 'http:' && location.protocol !== 'https:') return false;
    try {
      for (let i = 0; i < sessionStorage.length; i++) {
        if (sessionStorage.key(i).startsWith('hermes.webapp.session')) return true;
      }
    } catch {}
    return false;
  };
  window.addEventListener(
    'keydown',
    event => {
      if (event.repeat) return;
      if (event.isComposing || event.keyCode === 229) return;
      const reload =
        event.key === 'F5' ||
        ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R'));
      if (reload && canReload()) {
        event.preventDefault();
        location.reload();
      }
    },
    { capture: true }
  );

  // The shell's splash covers the window until this app has painted. Only the
  // backend page reports it: the bundled boot page has no native permission for
  // `app_painted`, and the splash is not covered by anything. Wait for load,
  // then for visible text, then for two frames, so the frame is really on
  // screen; the cap reports anyway, so a page that never fills in still lets
  // the shell's own cap take the splash away.
  if (/^https?:$/.test(location.protocol)) {
    const reportFirstPaint = () => {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          tauri.core.invoke('app_painted').catch(() => undefined);
        })
      );
    };
    const awaitFirstPaint = deadline => {
      if ((document.body?.innerText ?? '').trim() || Date.now() >= deadline) {
        reportFirstPaint();
        return;
      }
      setTimeout(() => awaitFirstPaint(deadline), 100);
    };
    window.addEventListener('load', () => awaitFirstPaint(Date.now() + 30000), { once: true });
  }

  let installedBridge;
  Object.defineProperty(window, 'hermesDesktop', {
    configurable: true,
    enumerable: true,
    get() {
      return installedBridge;
    },
    set(bridge) {
      installedBridge = bridge;
      if (bridge && typeof bridge === 'object') {
        bridge.openExternal = nativeOpenExternal;
        bridge.notify = nativeNotify;
        bridge.selectPaths = nativeSelectPaths;
      }
      Object.defineProperty(window, 'hermesDesktop', {
        configurable: true,
        enumerable: true,
        value: installedBridge,
        writable: true
      });
    }
  });
})();
