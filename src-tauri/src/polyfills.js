(() => {
  if (typeof window.requestIdleCallback === 'function') return;

  // WebKitGTK lacks requestIdleCallback/cancelIdleCallback (docs/webkit-compat.md).
  // Fires at 1 ms, so any requested `timeout` is honored; the idle budget of 50 ms
  // mirrors the fallbacks the app already uses for setTimeout-based shims.
  window.requestIdleCallback = callback => {
    const start = Date.now();
    return window.setTimeout(() => {
      callback({
        didTimeout: false,
        timeRemaining: () => Math.max(0, 50 - (Date.now() - start))
      });
    }, 1);
  };
  window.cancelIdleCallback = id => window.clearTimeout(id);
})();
