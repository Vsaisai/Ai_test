// Service worker: the app shell works offline; images and the API always go to the server.
const SHELL = 'vnshelf-shell-v14';
const FILES = ['./', 'index.html', 'app.css', 'js/app.js', 'js/api.js', 'js/ui.js', 'js/store.js', 'js/views.js', 'js/taxonomy.js', 'js/lightbox.js', 'js/tagtip.js', 'js/errors.js',
  'manifest.webmanifest', 'icons/icon.svg', 'icons/icon-192.png'];

self.addEventListener('install', e => {
  // cache: 'reload' so an update never stores modules from the browser's old HTTP cache
  e.waitUntil(caches.open(SHELL).then(c => c.addAll(FILES.map(f => new Request(f, { cache: 'reload' })))).then(() => self.skipWaiting()));
});
self.addEventListener('activate', e => {
  e.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(k => k !== SHELL).map(k => caches.delete(k))))
    .then(() => self.clients.claim()));
});
self.addEventListener('fetch', e => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET') return;
  // images: the server keeps its own size-limited cache and the browser caches them by HTTP headers
  if (url.pathname.includes('/img/')) return;
  if (url.pathname.includes('/api/')) return;
  // shell: network first, cache fallback
  e.respondWith(fetch(e.request).then(r => {
    if (r.ok && url.origin === location.origin) caches.open(SHELL).then(c => c.put(e.request, r.clone()));
    return r;
  }).catch(() => caches.match(e.request).then(r => r || caches.match('index.html'))));
});
