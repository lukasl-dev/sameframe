const helpers = await import(document.body.dataset.syncUrl).catch(() => {
  const error = document.getElementById('home-error');
  if (error) {
    error.hidden = false;
    error.textContent = 'The room client could not load. Reload this page to retry.';
  }

  for (const id of ['create-room', 'join-link']) {
    const node = document.getElementById(id);
    if (node) node.disabled = true;
  }
  document.getElementById('join-form')?.addEventListener('submit', (event) => event.preventDefault());
  return null;
});

if (helpers) {
  const { parseRoomLink } = helpers;
  const create = document.getElementById('create-room');
  const error = document.getElementById('home-error');
  let creating = false;

  function showError(message) {
    if (!error) return;
    error.textContent = message;
    error.hidden = !message;
  }

  create?.addEventListener('click', async (event) => {
    event.preventDefault();
    if (creating) return;

    creating = true;
    create.disabled = true;
    showError('');
    try {
      const response = await fetch('/api/rooms', { method: 'POST', headers: { Accept: 'application/json' } });
      if (!response.ok) throw new Error(response.status === 429 || response.status === 503
        ? 'Rooms are busy right now. Please try again shortly.' : 'Could not create a room. Please try again.');

      const data = await response.json();
      if (!/^[A-Za-z0-9_-]+$/.test(data.room_id ?? '') || typeof data.host_token !== 'string' || !data.host_token) {
        throw new Error('The server returned an invalid room. Please try again.');
      }
      location.assign(`/room/${data.room_id}#host=${encodeURIComponent(data.host_token)}`);
    } catch (problem) {
      showError(problem instanceof TypeError
        ? 'Could not reach the server. Check your connection and try again.' : problem.message);
    } finally {
      creating = false;
      create.disabled = false;
    }
  });

  document.getElementById('join-form')?.addEventListener('submit', (event) => {
    event.preventDefault();
    const path = parseRoomLink(document.getElementById('join-link')?.value, location.origin);
    if (!path) {
      showError('Paste a room link from this site, or enter its room code.');
      return;
    }
    location.assign(path);
  });
}
