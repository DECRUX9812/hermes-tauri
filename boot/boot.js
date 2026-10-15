(() => {
  const headline = document.querySelector('#headline');
  const detail = document.querySelector('#detail');
  const error = document.querySelector('#error');
  const retry = document.querySelector('#retry');
  const tauri = window.__TAURI__;

  const render = state => {
    retry.hidden = true;
    error.hidden = true;
    error.textContent = '';

    switch (state.phase) {
      case 'no_config':
        headline.textContent = 'No Hermes configured';
        detail.textContent =
          'Pass --url or set HERMES_TAURI_URL to attach to a running server, ' +
          'or set HERMES_TAURI_CMD or a config.toml to start one.';
        return;
      case 'preparing':
        // The bundled payload's first run: unpacking takes seconds, and the
        // window must say so instead of staying blank.
        headline.textContent = 'Preparing Hermes (first run)…';
        detail.textContent = 'Unpacking the bundled Hermes payload. This happens once.';
        return;
      case 'probing':
        headline.textContent = 'Connecting to Hermes…';
        detail.textContent = state.origin
          ? `Checking ${state.origin}`
          : 'Checking the configured Hermes server.';
        return;
      case 'starting':
        headline.textContent = 'Starting Hermes…';
        detail.textContent = 'Launching the configured Hermes server.';
        return;
      case 'waiting':
        headline.textContent = 'Waiting for Hermes to become ready…';
        detail.textContent = state.origin
          ? `Health-checking ${state.origin}`
          : 'Health-checking the new Hermes server.';
        return;
      case 'ready':
        headline.textContent = 'Hermes is ready';
        detail.textContent = 'Opening the app…';
        return;
      default:
        headline.textContent = state.origin
          ? `Hermes not reachable at ${state.origin}`
          : 'Hermes not reachable';
        detail.textContent = 'Check that the server is running, then try again.';
        if (state.error) {
          error.textContent = state.error;
          error.hidden = false;
        }
        retry.hidden = false;
    }
  };

  const showFailure = failure => render({
    error: failure instanceof Error ? failure.message : String(failure),
    origin: null,
    phase: 'unreachable'
  });

  const syncState = () => tauri.core.invoke('boot_state').then(render);

  retry.addEventListener('click', async () => {
    retry.disabled = true;
    try {
      await tauri.core.invoke('boot_retry');
      await syncState();
    } catch (failure) {
      showFailure(failure);
    } finally {
      retry.disabled = false;
    }
  });

  if (!tauri?.core?.invoke) {
    showFailure(new Error('Tauri boot commands are unavailable.'));
    return;
  }
  // If the event permission is denied, the state is still read through boot_state.
  tauri.event.listen('boot-state', event => render(event.payload)).catch(() => undefined);
  syncState().catch(showFailure);
})();
