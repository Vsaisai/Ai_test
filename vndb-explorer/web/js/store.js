// Per-device settings (localStorage). Personal lists live on the server.
const KEY = 'vnshelf.settings';

export const DEFAULTS = {
  apiBase: '',            // '' = same server; e.g. 'http://192.168.1.10:8080' for a mobile app
  langs: 'en',            // preferred languages, comma separated, in order
  langMode: 'readable',   // readable | list | any
  allowMtl: true,         // count machine translations
  adult: true,            // show 18+ shelves, tags and VNs
  blur: 'explicit',       // off | explicit | suggestive  (blur covers & screenshots)
  spoiler: 0,             // 0 none, 1 minor, 2 all
  theme: 'auto',          // auto | dark | light
  hideCancelled: false,
};

let cache = null;

export function settings() {
  if (!cache) {
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch { /* private mode */ }
    cache = { ...DEFAULTS, ...saved };
  }
  return cache;
}

export function save(patch) {
  cache = { ...settings(), ...patch };
  try { localStorage.setItem(KEY, JSON.stringify(cache)); } catch { /* ignore */ }
  applyTheme();
  return cache;
}

export function langList() {
  return settings().langs.split(',').map(s => s.trim()).filter(Boolean);
}

/** Base search request built from the user's preferences. */
export function baseRequest() {
  const s = settings();
  return {
    lang_mode: s.langMode,
    langs: langList().length ? langList() : ['en'],
    allow_mtl: s.allowMtl,
    ero: s.adult ? 'show' : 'hide',
    spoiler: Number(s.spoiler) || 0,
    hide_cancelled: s.hideCancelled,
  };
}

export function applyTheme() {
  const t = settings().theme;
  document.documentElement.dataset.theme = t === 'auto' ? '' : t;
}
