// Entry point: navigation + hash router.
import { applyTheme, settings, save, IN_APP, APP_VERSION, RELEASES_PAGE } from './store.js';
import { h, jobPanel, progressLine } from './ui.js';
import { installTagTips } from './tagtip.js';
import { api } from './api.js';
import { humanError } from './errors.js';
import { viewProfile } from './profile.js';
import { openFilters, closeFilters } from './filters.js';
import { viewHome, viewShelf, viewSearch, viewTags, viewTag, viewVn, viewMy, viewSettings } from './views.js';

// One entry per place: the tag catalog is part of the search, the logo is not a second "home" button.
const NAV = [
  ['#/my', '❤️', 'Мои'],
  ['#/', '📚', 'Главная'],
  ['#/profile', '👤', 'Профиль'],
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
    // search, tags and VN pages belong to "Главная"; settings to the profile
    const on = href === '#/' ? ['#/', '#/shelf', '#/vn', '#/search', '#/tags', '#/tag'].includes(section)
      : section.startsWith(href) || (href === '#/profile' && section === '#/settings');
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
  closeFilters();
  // the top search field shows the title of the current search
  document.getElementById('topq').value = parts[0] === 'search' ? (params.get('q') || '') : '';
  document.querySelector('.filter-dot').hidden = !(parts[0] === 'search' && [...params.keys()].some(k => k !== 'q' && k !== 'sort'));
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
    case 'tags': return viewTags(target, parts.slice(1), params);
    case 'tag': return viewTag(target, parts[1]);
    case 'vn': return viewVn(target, parts[1], params);
    case 'my': return viewMy(target, parts[1]);
    case 'settings': return viewSettings(target);
    case 'profile': return viewProfile(target);
    default: view.append(h('p', {}, 'Страница не найдена. ', h('a', { href: '#/' }, 'На главную')));
  }
}

document.getElementById('topsearch').addEventListener('submit', e => {
  e.preventDefault();
  const q = document.getElementById('topq').value.trim();
  // on the results page only the title changes; the filters stay
  const [path, query = ''] = location.hash.slice(1).split('?');
  const params = path === '/search' ? new URLSearchParams(query) : new URLSearchParams();
  if (q) params.set('q', q); else params.delete('q');
  location.hash = `#/search?${params}`;
  document.getElementById('topq').blur();
});
document.getElementById('filterbtn').addEventListener('click', () => openFilters());

// ---------- updates ----------

const banner = h('div', { class: 'update-banner', hidden: true });
document.body.insertBefore(banner, document.getElementById('view'));

/** A message box writer: (text, actions) -> shows them in `el` with a "Hide" link. */
export function bannerOut(el, { closable = true } = {}) {
  return (text, actions = []) => {
    const kids = [typeof text === 'string' ? h('span', {}, text) : text, ...actions];
    if (closable) kids.push(h('button', { class: 'linkish', onclick: () => { el.hidden = true; } }, 'Скрыть'));
    el.replaceChildren(...kids.filter(Boolean));
    el.hidden = false;
  };
}

// Two separate boxes: database jobs (progress every second) must not overwrite an app update offer.
const appBanner = h('div', { class: 'update-banner', hidden: true });
document.body.insertBefore(appBanner, banner);
const showAppBanner = bannerOut(appBanner);

function showBanner(text, actions = []) {
  bannerOut(banner)(text, actions);
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
    if (job.error) { showBanner(`⚠️ Обновление не удалось: ${humanError(job.error)}`); return; }
    // the data changed: redraw the current page by itself instead of asking to reload
    api.resetTags();
    route();
    const text = doneText || ({ import: '✅ База VNDB загружена.', sync: '✅ Новые новеллы и релизы добавлены.', sources: '✅ Каталоги переводов обновлены.' })[job.kind] || '✅ Готово.';
    showBanner(text);
    const shown = banner.textContent;
    setTimeout(() => { if (banner.textContent === shown) banner.hidden = true; }, 6000);
    return;
  }
}

/** Is the local database behind VNDB? Small gaps are fetched through the API, big ones need a full
 *  download. `show`: where to write the result (default: the banner on top); `onStart`: called when
 *  a job starts (the settings page shows its progress itself). */
export async function checkUpdates({ force = false, interactive = false, show = null, onStart = null } = {}) {
  const s = settings();
  const out = show || showBanner;
  if (!interactive && s.updatePolicy === 'manual') return null;
  if (!force && Date.now() - (s.lastUpdateCheck || 0) < 6 * 3600 * 1000) return null;
  let r;
  try { r = await api.updates(force); } catch (e) { if (interactive) out(`⚠️ ${e.message}`); return null; }
  if (!r.ready) return r;
  save({ lastUpdateCheck: Date.now() });
  const c = r.check;
  const fresh = c.new_vns + c.new_releases;
  const start = async (call, running, done) => {
    await call();
    if (onStart) { out(running); onStart(); } else { out(running); watchJob(done); }
  };
  const full = () => start(api.startImport, '⏳ Скачиваю свежую базу VNDB…', '✅ База полностью обновлена.');
  const sync = () => start(api.sync, `⏳ Добавляю ${c.new_vns} новелл и ${c.new_releases} релизов…`,
    `✅ Добавлено новелл: ${c.new_vns}, релизов: ${c.new_releases}.`);
  if (c.big) {
    if (s.updatePolicy === 'auto' && !interactive) { full(); return r; }
    out(`База сильно отстала от VNDB (${reasonRu(c)}). Нужна полная загрузка (~190 МБ).`,
      [h('button', { class: 'btn primary', onclick: full }, 'Обновить сейчас')]);
  } else if (fresh > 0) {
    if (!interactive) sync();
    else out(`На VNDB появилось ${c.new_vns} новелл и ${c.new_releases} релизов.`, [h('button', { class: 'btn primary', onclick: sync }, 'Подтянуть')]);
  } else if (interactive) {
    out('✅ База актуальна: новых новелл и релизов на VNDB нет.');
  }
  return r;
}

function reasonRu(c) {
  if (c.new_vns > 1500) return `новых новелл: ${c.new_vns}`;
  if (c.new_releases > 6000) return `новых релизов: ${c.new_releases}`;
  return `последняя полная загрузка ${Math.round(c.days_since_import)} дн. назад`;
}

// ---------- app updates (Android build only) ----------

function newer(a, b) {
  const pa = a.split('.').map(Number); const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0);
  }
  return false;
}

// Releases of the Android app (public repository, no token needed).
const RELEASES_API = 'https://api.github.com/repos/Vsaisai/vnshelf-releases/releases?per_page=20';

/** The newest `mobile-v*` release: {version, size, url, page} or null. */
async function latestRelease() {
  let r;
  try {
    r = await fetch(RELEASES_API, { headers: { Accept: 'application/vnd.github+json' } });
  } catch {
    throw new Error('Нет связи с GitHub. Проверьте интернет и повторите.');
  }
  if (r.status === 403 || r.status === 429) throw new Error('GitHub временно ограничил проверки — попробуйте через час.');
  if (!r.ok) throw new Error(`GitHub: HTTP ${r.status}`);
  const rel = (await r.json()).find(x => !x.draft && !x.prerelease && /^mobile-v/.test(x.tag_name));
  if (!rel) return null;
  const apk = (rel.assets || []).find(a => a.name.endsWith('.apk'));
  return { version: rel.tag_name.replace(/^mobile-v/, ''), size: apk?.size, url: apk?.browser_download_url, page: rel.html_url };
}

/** Check for a newer app version. `show`: where to write the result (default: the banner on top). */
export async function checkAppUpdate({ interactive = false, show = null } = {}) {
  if (!IN_APP) return null;
  const out = show || showAppBanner;
  if (!interactive && Date.now() - (settings().lastAppCheck || 0) < 24 * 3600 * 1000) return null;
  // remember the attempt even when it fails: GitHub limits anonymous checks to 60 per hour
  save({ lastAppCheck: Date.now() });
  let rel;
  try {
    rel = await latestRelease();
  } catch (e) {
    if (interactive) out(`⚠️ ${e.message}`, [h('a', { class: 'btn', href: RELEASES_PAGE }, 'Скачать вручную')]);
    return null;
  }
  if (!rel || !rel.url) {
    if (interactive) out('Релизов приложения пока нет.');
  } else if (newer(rel.version, APP_VERSION)) {
    const size = rel.size ? ` (${(rel.size / 1048576).toFixed(1).replace('.', ',')} МБ)` : '';
    const canSelf = !!window.VNShelfNative?.startUpdate;
    out(`⬆️ Доступна версия ${rel.version} (у вас ${APP_VERSION}).`, [
      canSelf
        ? h('button', { class: 'btn primary', onclick: () => selfUpdate(rel.url, rel.version, out) }, `Обновить${size}`)
        : h('a', { class: 'btn primary', href: rel.url }, `Скачать APK${size}`),
      h('a', { class: 'btn', href: rel.page || RELEASES_PAGE }, 'Что нового')]);
  } else if (interactive) {
    out(`✅ У вас последняя версия ${APP_VERSION}.`);
  }
  return rel;
}

/** Download the new APK inside the app and hand it to the system installer (like Telegram). */
function selfUpdate(url, ver, out = showAppBanner) {
  const res = window.VNShelfNative.startUpdate(url);
  if (res === 'need_permission') {
    out('Разрешите приложению устанавливать обновления (откроются настройки Android), затем нажмите «Обновить» ещё раз.',
      [h('button', { class: 'btn primary', onclick: () => selfUpdate(url, ver, out) }, 'Обновить')]);
    return;
  }
  const poll = () => {
    let st = {};
    try { st = JSON.parse(window.VNShelfNative.updateStatus() || '{}'); } catch { /* ignore */ }
    if (st.state === 'downloading') {
      // total is -1 when the size is unknown
      const known = st.total > 0;
      const pct = known ? Math.min(100, Math.max(0, Math.floor(st.done * 100 / st.total))) : null;
      const mb = b => (Math.max(0, b) / 1048576).toFixed(1).replace('.', ',');
      out(progressLine(pct, `Загружаю версию ${ver} · ${known ? `${mb(st.done)} из ${mb(st.total)} МБ` : `скачано ${mb(st.done)} МБ`}`));
      setTimeout(poll, 400);
    } else if (st.state === 'installing') {
      out(`Подтвердите установку версии ${ver} в системном окне.`);
      setTimeout(poll, 1000);
    } else if (st.state === 'error') {
      out(`⚠️ Не удалось обновить: ${st.error || 'ошибка'}`, [h('button', { class: 'btn', onclick: () => selfUpdate(url, ver, out) }, 'Повторить')]);
    } else if (st.state === 'done') {
      out('Обновление установлено — приложение перезапустится.');
    } else {
      setTimeout(poll, 500);
    }
  };
  poll();
}

// bottom menu in the app, on touch screens (phones, tablets) and narrow windows; top menu on desktops
const touchNav = () => IN_APP || matchMedia('(pointer: coarse)').matches || innerWidth <= 760;
document.documentElement.classList.toggle('touch-nav', touchNav());
addEventListener('resize', () => document.documentElement.classList.toggle('touch-nav', touchNav()));

applyTheme();
buildNav();
installTagTips();
window.addEventListener('hashchange', route);
route();
setTimeout(() => checkUpdates(), 1500);
// a job may already be running (e.g. the app was reopened during a download)
api.status().then(st => { if (st.ready && st.import?.running) watchJob(); }).catch(() => {});
setTimeout(() => checkAppUpdate(), 4000);

if ('serviceWorker' in navigator && location.protocol !== 'file:') {
  navigator.serviceWorker.register('sw.js').catch(() => {});
}
