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

    if (state.phase === 'no_config') {
      headline.textContent = 'No Hermes configured';
      detail.textContent = 'Pass --url or set HERMES_TAURI_URL to connect to a Hermes server.';
      return;
    }
    if (state.phase === 'unreachable') {
      headline.textContent = `Hermes not reachable at ${state.origin || 'the configured URL'}`;
      detail.textContent = 'Check that the server is running, then try again.';
      if (state.error) {
        error.textContent = state.error;
        error.hidden = false;
      }
      retry.hidden = false;
      return;
    }

    headline.textContent = 'Connecting to Hermes…';
    detail.textContent = state.origin
      ? `Checking ${state.origin}`
      : 'Checking the configured Hermes server.';
  };

  retry.addEventListener('click', async () => {
    retry.disabled = true;
    try {
      await tauri.core.invoke('boot_retry');
    } catch (failure) {
      render({
        error: failure instanceof Error ? failure.message : String(failure),
        origin: null,
        phase: 'unreachable'
      });
    } finally {
      retry.disabled = false;
    }
  });

  const showFailure = failure => render({
    error: failure instanceof Error ? failure.message : String(failure),
    origin: null,
    phase: 'unreachable'
  });

  if (!tauri?.core?.invoke) {
    showFailure(new Error('Tauri boot commands are unavailable.'));
    return;
  }
  Promise.resolve(tauri.event.listen('boot-state', event => render(event.payload)))
    .then(() => tauri.core.invoke('boot_state'))
    .then(render)
    .catch(showFailure);
})();
