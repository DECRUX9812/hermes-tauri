(() => {
  const backendOrigin = __HERMES_BACKEND_ORIGIN__;
  if (location.origin !== backendOrigin) return;

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
