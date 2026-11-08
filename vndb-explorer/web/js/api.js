// Transport layer. Everything talks to the REST API through here, so a mobile shell
// (Capacitor / Tauri) only needs to change `apiBase` or replace `call`.
import { settings, langList } from './store.js';
import { humanError } from './errors.js';

const base = () => (settings().apiBase || '').replace(/\/$/, '');

async function call(method, path, body) {
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  let r;
  try {
    r = await fetch(base() + path, opts);
  } catch (e) {
    throw new Error(settings().apiBase
      ? 'Нет связи с сервером. Проверьте, что он запущен и адрес в настройках верный.'
      : 'Нет связи с приложением. Перезапустите его, если ошибка повторяется.');
  }
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(humanError(data.error || `HTTP ${r.status}`));
  return data;
}

export const img = path => path ? `${base()}/img/${path}` : '';

let tagsPromise = null;
let translateDownUntil = 0;

export const api = {
  async status() {
    const st = await call('GET', '/api/status');
    // Android app: keep the screen on while a download / import runs (sleep would cut the connection)
    try {
      const running = !!st.import?.running;
      window.VNShelfNative?.keepAwake(running);
      // Android: hand the job over to a foreground service with a progress notification
      if (running) window.VNShelfNative?.watchJob();
    } catch { /* not in the app */ }
    return st;
  },
  startImport: () => call('POST', '/api/import', {}),
  updates: (force = false) => call('GET', `/api/updates${force ? '?force=true' : ''}`),
  async translate(texts, to = 'ru') {
    // after a refusal (rate limit) give the free translators a break instead of asking on every page
    if (Date.now() < translateDownUntil) throw new Error('Переводчик временно недоступен — показан оригинал.');
    try {
      return await call('POST', '/api/translate', { texts, to });
    } catch (e) {
      translateDownUntil = Date.now() + 120_000;
      throw e;
    }
  },
  sync: () => call('POST', '/api/sync', {}),
  sources: () => call('GET', '/api/sources'),
  refreshSources: () => call('POST', '/api/sources/refresh', {}),
  tags() {
    if (!tagsPromise) {
      tagsPromise = call('GET', '/api/tags').then(d => {
        const byId = new Map(d.tags.map(t => [t.id, t]));
        return { roots: d.roots, list: d.tags, byId };
      }).catch(e => { tagsPromise = null; throw e; });
    }
    return tagsPromise;
  },
  resetTags() { tagsPromise = null; },
  tag: id => call('GET', `/api/tags/${id}`),
  search: req => call('POST', '/api/search', req),
  langs: () => call('GET', '/api/langs'),
  shelvesConfig: () => call('GET', '/api/shelves-config'),
  shelves: (base, adult, perShelf = 16) => call('POST', '/api/shelves', { base, adult, per_shelf: perShelf }),
  shelf: (id, base, sort, offset, limit = 48) => call('POST', `/api/shelves/${encodeURIComponent(id)}`, { base, sort, offset, limit }),
  random: req => call('POST', '/api/random', req),
  vn: id => call('GET', `/api/vn/${id}?langs=${encodeURIComponent(langList().join(','))}&any_lang=false`),
  prices: (id, cc) => call('GET', `/api/vn/${id}/prices?cc=${encodeURIComponent(cc)}`),
  my: () => call('GET', '/api/my'),
  profile: () => call('GET', '/api/profile'),
  saveProfile: p => call('PUT', '/api/profile', p),
  mark: (id, status, note) => call('PUT', `/api/my/${id}`, { status, note }),
  unmark: id => call('DELETE', `/api/my/${id}`),
};
