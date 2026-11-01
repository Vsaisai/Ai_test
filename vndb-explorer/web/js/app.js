// Entry point: navigation + hash router.
import { applyTheme } from './store.js';
import { h } from './ui.js';
import { viewHome, viewShelf, viewSearch, viewTags, viewTag, viewVn, viewMy, viewSettings } from './views.js';

const NAV = [
  ['#/', '📚', 'Полки'],
  ['#/search', '🔎', 'Поиск'],
  ['#/tags', '🏷️', 'Теги'],
  ['#/my', '❤️', 'Мои'],
  ['#/settings', '⚙️', 'Настройки'],
];

function buildNav() {
  const items = NAV.map(([href, icon, label]) => h('a', { href, 'data-href': href }, h('span', { class: 'ico' }, icon), h('span', { class: 'lbl' }, label)));
  document.getElementById('mainnav').replaceChildren(...items);
  document.getElementById('bottomnav').replaceChildren(...items.map(a => a.cloneNode(true)));
}

function highlightNav(path) {
  const section = '#/' + (path.split('/')[1] || '');
  document.querySelectorAll('[data-href]').forEach(a => {
    const href = a.dataset.href;
    const on = href === '#/' ? ['#/', '#/shelf', '#/vn'].includes(section) : section.startsWith(href) || (href === '#/tags' && section === '#/tag');
    a.classList.toggle('on', on);
  });
}

let token = 0;

async function route() {
  const raw = location.hash.slice(1) || '/';
  const [path, query = ''] = raw.split('?');
  const params = new URLSearchParams(query);
  const parts = path.split('/').filter(Boolean);
  const root = document.getElementById('view');
  const my = ++token;
  const view = h('div', { class: 'view' });
  root.replaceChildren(view);
  document.title = 'VN Shelf';
  highlightNav(path);
  window.scrollTo(0, 0);
  const guard = el => (my === token ? el : document.createElement('div'));
  const target = guard(view);
  switch (parts[0]) {
    case undefined: return viewHome(target);
    case 'shelf': return viewShelf(target, parts[1], params);
    case 'search': return viewSearch(target, params);
    case 'tags': return viewTags(target, parts[1], params);
    case 'tag': return viewTag(target, parts[1]);
    case 'vn': return viewVn(target, parts[1]);
    case 'my': return viewMy(target, parts[1]);
    case 'settings': return viewSettings(target);
    default: view.append(h('p', {}, 'Страница не найдена. ', h('a', { href: '#/' }, 'На главную')));
  }
}

document.getElementById('quicksearch').addEventListener('submit', e => {
  e.preventDefault();
  const q = document.getElementById('quickq').value.trim();
  location.hash = q ? `#/search?q=${encodeURIComponent(q)}` : '#/search';
});

applyTheme();
buildNav();
window.addEventListener('hashchange', route);
route();

if ('serviceWorker' in navigator && location.protocol !== 'file:') {
  navigator.serviceWorker.register('sw.js').catch(() => {});
}
