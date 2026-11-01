// Transport layer. Everything talks to the REST API through here, so a mobile shell
// (Capacitor / Tauri) only needs to change `apiBase` or replace `call`.
import { settings, langList } from './store.js';

const base = () => (settings().apiBase || '').replace(/\/$/, '');

async function call(method, path, body) {
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const r = await fetch(base() + path, opts);
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(data.error || `HTTP ${r.status}`);
  return data;
}

export const img = path => path ? `${base()}/img/${path}` : '';

let tagsPromise = null;

export const api = {
  status: () => call('GET', '/api/status'),
  startImport: () => call('POST', '/api/import', {}),
  updates: (force = false) => call('GET', `/api/updates${force ? '?force=1' : ''}`),
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
  shelves: (base, adult, perShelf = 16) => call('POST', '/api/shelves', { base, adult, per_shelf: perShelf }),
  shelf: (id, base, sort, offset, limit = 48) => call('POST', `/api/shelves/${encodeURIComponent(id)}`, { base, sort, offset, limit }),
  random: req => call('POST', '/api/random', req),
  vn: id => call('GET', `/api/vn/${id}?langs=${encodeURIComponent(langList().join(','))}&any_lang=${settings().langMode === 'any'}`),
  prices: (id, cc) => call('GET', `/api/vn/${id}/prices?cc=${encodeURIComponent(cc)}`),
  my: () => call('GET', '/api/my'),
  mark: (id, status, note) => call('PUT', `/api/my/${id}`, { status, note }),
  unmark: id => call('DELETE', `/api/my/${id}`),
};
