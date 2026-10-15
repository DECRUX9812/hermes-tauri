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
