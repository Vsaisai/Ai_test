// Entry point: navigation + hash router.
import { applyTheme, settings, save } from './store.js';
import { h, toast, jobPanel } from './ui.js';
import { api } from './api.js';
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

// ---------- updates ----------

const banner = h('div', { class: 'update-banner', hidden: true });
document.body.insertBefore(banner, document.getElementById('view'));

export function showBanner(text, actions = []) {
  banner.replaceChildren(typeof text === 'string' ? h('span', {}, text) : text, ...actions, h('button', { class: 'linkish', onclick: () => { banner.hidden = true; } }, 'Скрыть'));
  banner.hidden = false;
}

async function watchJob(doneText) {
  for (;;) {
    await new Promise(r => setTimeout(r, 1000));
    const st = await api.status().catch(() => null);
    if (!st) continue;
    const job = st.import;
    if (job.running) {
      showBanner(jobPanel(job, { compact: true, link: '#/settings' }));
      continue;
    }
    if (job.error) showBanner(`⚠️ Обновление не удалось: ${job.error}`);
    else { showBanner(doneText, [h('button', { class: 'btn', onclick: () => location.reload() }, 'Обновить страницу')]); api.resetTags(); }
    return;
  }
}

export async function checkUpdates({ force = false, interactive = false } = {}) {
  const s = settings();
  if (!interactive && s.updatePolicy === 'manual') return null;
  if (!force && Date.now() - (s.lastUpdateCheck || 0) < 6 * 3600 * 1000) return null;
  let r;
  try { r = await api.updates(force); } catch (e) { if (interactive) toast(`Нет связи с VNDB: ${e.message}`); return null; }
  if (!r.ready) return r;
  save({ lastUpdateCheck: Date.now() });
  const c = r.check;
  const fresh = c.new_vns + c.new_releases;
  const full = async () => { await api.startImport(); showBanner('⏳ Скачиваю свежую базу VNDB…'); watchJob('✅ База полностью обновлена.'); };
  if (c.big) {
    if (s.updatePolicy === 'auto' && !interactive) { full(); return r; }
    showBanner(`База сильно отстала от VNDB (${reasonRu(c)}). Нужна полная загрузка (~190 МБ).`,
      [h('button', { class: 'btn primary', onclick: full }, 'Обновить сейчас')]);
  } else if (fresh > 0) {
    const go = async () => { await api.sync(); showBanner(`⏳ Добавляю ${c.new_vns} новелл и ${c.new_releases} релизов…`); watchJob(`✅ Добавлено новелл: ${c.new_vns}, релизов: ${c.new_releases}.`); };
    if (!interactive) go();
    else showBanner(`На VNDB появилось ${c.new_vns} новелл и ${c.new_releases} релизов.`, [h('button', { class: 'btn primary', onclick: go }, 'Подтянуть')]);
  } else if (interactive) {
    toast('База актуальна');
  }
  return r;
}

function reasonRu(c) {
  if (c.new_vns > 1500) return `новых новелл: ${c.new_vns}`;
  if (c.new_releases > 6000) return `новых релизов: ${c.new_releases}`;
  return `последняя полная загрузка ${Math.round(c.days_since_import)} дн. назад`;
}

applyTheme();
buildNav();
window.addEventListener('hashchange', route);
route();
setTimeout(() => checkUpdates(), 1500);
// a job may already be running (e.g. the app was reopened during a download)
api.status().then(st => { if (st.ready && st.import?.running) watchJob('✅ Готово.'); }).catch(() => {});

if ('serviceWorker' in navigator && location.protocol !== 'file:') {
  navigator.serviceWorker.register('sw.js').catch(() => {});
}
