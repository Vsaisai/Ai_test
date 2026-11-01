// Service worker: app shell works offline, cover images are cached, API always goes to the server.
const SHELL = 'vnshelf-shell-v1';
const IMAGES = 'vnshelf-img-v1';
const FILES = ['./', 'index.html', 'app.css', 'js/app.js', 'js/api.js', 'js/ui.js', 'js/store.js', 'js/views.js',
  'manifest.webmanifest', 'icons/icon.svg', 'icons/icon-192.png'];

self.addEventListener('install', e => {
  e.waitUntil(caches.open(SHELL).then(c => c.addAll(FILES)).then(() => self.skipWaiting()));
});
self.addEventListener('activate', e => {
  e.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(k => ![SHELL, IMAGES].includes(k)).map(k => caches.delete(k))))
    .then(() => self.clients.claim()));
});
self.addEventListener('fetch', e => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET') return;
  if (url.pathname.includes('/img/')) {
    e.respondWith(caches.open(IMAGES).then(async c => {
      const hit = await c.match(e.request);
      if (hit) return hit;
      const r = await fetch(e.request);
      if (r.ok) c.put(e.request, r.clone());
      return r;
    }));
    return;
  }
  if (url.pathname.includes('/api/')) return;
  // shell: network first, cache fallback
  e.respondWith(fetch(e.request).then(r => {
    if (r.ok && url.origin === location.origin) caches.open(SHELL).then(c => c.put(e.request, r.clone()));
    return r;
  }).catch(() => caches.match(e.request).then(r => r || caches.match('index.html'))));
});
