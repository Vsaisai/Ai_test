// Per-device settings (localStorage). Personal lists live on the server.
const KEY = 'vnshelf.settings';

/** Running inside the Android app (WebView with a custom user agent)? */
export const APP_VERSION = (navigator.userAgent.match(/VNShelfApp\/([\w.\-]+)/) || [])[1] || null;
export const IN_APP = APP_VERSION !== null;
/** Where mobile releases are published. */
export const RELEASES_PAGE = 'https://gitlab.com/Asgefka/Ai_test/-/releases';
export const ANDROID_BUILDS = 'https://gitlab.com/Asgefka/Ai_test/-/jobs/artifacts/mobile/browse?job=android-apk';

export const DEFAULTS = {
  apiBase: '',            // '' = same server; e.g. 'http://192.168.1.10:8080' for a mobile app
  langs: 'en',            // preferred languages, comma separated, in order
  langMode: 'readable',   // readable | list | any
  allowMtl: true,         // count machine translations
  adultTags: false,       // adult tags in the catalog, search and VN pages (+ their shelves)
  hideAdult: false,       // hide VNs with sexual content from lists
  blur: 'explicit',       // off | explicit | suggestive  (blur covers & screenshots)
  spoiler: 0,             // 0 none, 1 minor, 2 all
  theme: 'auto',          // auto | dark | light
  hideCancelled: false,
  updatePolicy: 'smart',  // smart: small updates automatically, ask before a full re-download
                          // auto: everything automatically | manual: only from Settings
  platform: 'any',        // any | pc | phone — where the game should be playable
  translate: true,
  f95: false,             // F95Zone catalog (only with adultTags)        // VN pages: descriptions and tags in Russian (machine translation, cached)
  lastUpdateCheck: 0,
  lastAppCheck: 0,
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

/** Platforms that count as "PC" / "phone". */
export const PLATFORM_GROUPS = {
  pc: ['win', 'lin', 'mac'],
  phone: ['and', 'ios'],
  console: ['swi', 'ps5', 'ps4', 'psv', 'ps3', 'psp', 'ps2', 'xbs', 'xb1', 'n3d', 'nds', 'drc'],
};

export function langList() {
  return settings().langs.split(',').map(s => s.trim()).filter(Boolean);
}

/** Base search request built from the user's preferences. */
export function baseRequest() {
  const s = settings();
  return {
    // which languages and platforms: chosen in the search (defaults: readable, anywhere)
    lang_mode: 'readable',
    langs: langList().length ? langList() : ['en'],
    allow_mtl: s.allowMtl,
    ero: s.hideAdult ? 'hide' : 'show',
    spoiler: Number(s.spoiler) || 0,
    hide_cancelled: s.hideCancelled,
    platforms: [],
  };
}

export function applyTheme() {
  const t = settings().theme;
  document.documentElement.dataset.theme = t === 'auto' ? '' : t;
}
