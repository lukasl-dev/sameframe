const storageKey = 'sameframe:access-token';
const input = document.getElementById('access-token');
const submit = document.getElementById('unlock-access');
const visibility = document.getElementById('access-visibility');
const status = document.getElementById('access-status');
const error = document.getElementById('access-error');
const form = document.getElementById('access-form');

const url = new URL(location.href);
const supplied = url.searchParams.get('access_token');
const hasSupplied = url.searchParams.has('access_token');
url.searchParams.delete('access_token');
if (hasSupplied) history.replaceState(null, '', url.href);

let saved = null;
try { saved = localStorage.getItem(storageKey); } catch {}

let busy = false;

function problem(message) {
  error.textContent = message;
  error.hidden = !message;
}

function controls(disabled) {
  input.disabled = disabled;
  submit.disabled = disabled;
  visibility.disabled = disabled;
}

async function unlock(token, restoring = false) {
  if (busy) return;

  if (!/^[0-9a-f]{64}$/.test(token)) {
    status.textContent = '';
    problem('That key doesn’t look quite right. Paste the full access token from your invite.');
    controls(false);
    input.focus();
    return;
  }

  busy = true;
  controls(true);
  problem('');
  status.textContent = restoring ? 'Checking your saved key…' : 'Opening the door…';

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await fetch('/api/access', {
      method: 'POST',
      credentials: 'same-origin',
      cache: 'no-store',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ token }),
      signal: controller.signal,
    });

    if (!response.ok) {
      if (response.status === 403 && restoring) {
        try { localStorage.removeItem(storageKey); } catch {}
      }
      problem(response.status === 429 ? 'A few too many knocks. Give it a moment, then try again.'
        : response.status === 403 ? 'That key didn’t open the door. Ask your people for the current access token.'
          : 'The door is a little stuck. Please try again shortly.');
      status.textContent = '';
      return;
    }

    const verified = await fetch('/api/access', {
      credentials: 'same-origin',
      cache: 'no-store',
      headers: { Accept: 'application/json' },
      signal: controller.signal,
    });
    if (!verified.ok) {
      status.textContent = '';
      problem('Your browser couldn’t remember the key. Allow cookies for this site, then try again.');
      return;
    }

    try { localStorage.setItem(storageKey, token); } catch {}
    input.value = '';
    status.textContent = 'You’re in. Make yourself at home.';
    location.reload();
  } catch {
    status.textContent = '';
    problem('We couldn’t reach the nook. Check your connection and try again.');
  } finally {
    clearTimeout(timeout);
    busy = false;
    controls(false);
  }
}

form.addEventListener('submit', (event) => {
  event.preventDefault();
  unlock(input.value.trim());
});

visibility.addEventListener('click', () => {
  const show = input.type === 'password';
  input.type = show ? 'text' : 'password';
  visibility.textContent = show ? 'Hide' : 'Show';
  visibility.setAttribute('aria-pressed', String(show));
});

controls(false);
status.textContent = '';
if (hasSupplied) await unlock(supplied ?? '');
else if (saved) await unlock(saved, true);
