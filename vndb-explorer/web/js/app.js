// Entry point: navigation + hash router.
import { applyTheme, settings, save, IN_APP, APP_VERSION, RELEASES_PAGE } from './store.js';
import { h, toast, jobPanel } from './ui.js';
import { installTagTips } from './tagtip.js';
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
    case 'tags': return viewTags(target, parts.slice(1), params);
    case 'tag': return viewTag(target, parts[1]);
    case 'vn': return viewVn(target, parts[1], params);
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

export function showBanner(text, actions = []) {
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
  try { r = await api.updates(force); } catch (e) { if (interactive) toast(e.message); return null; }
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

// ---------- app updates (Android build only) ----------

function newer(a, b) {
  const pa = a.split('.').map(Number); const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0);
  }
  return false;
}

const PUBLIC_RELEASES = 'https://api.github.com/repos/Vsaisai/vnshelf-releases/releases?per_page=20';

/** Latest release read by the WebView itself (fallback when the local server is offline). */
export async function directRelease() {
  const r = await fetch(PUBLIC_RELEASES, { headers: { Accept: 'application/vnd.github+json' } });
  if (!r.ok) throw new Error(`GitHub: HTTP ${r.status}`);
  const rel = (await r.json()).find(x => !x.draft && !x.prerelease && /^mobile-v/.test(x.tag_name));
  if (!rel) return { status: 'ok', version: null };
  const apk = (rel.assets || []).find(a => a.name.endsWith('.apk'));
  return { status: 'ok', repo: 'public', version: rel.tag_name.replace(/^mobile-v/, ''), html_url: rel.html_url,
    size: apk?.size, public_url: apk?.browser_download_url };
}

/** `show`: where to write the result (default: the app-update banner at the top). */
export async function checkAppUpdate({ interactive = false, show = null } = {}) {
  const out = show || showAppBanner;
  if (!IN_APP) return null;
  const s = settings();
  if (!interactive && Date.now() - (s.lastAppCheck || 0) < 24 * 3600 * 1000) return null;
  try {
    let u;
    try { u = await api.appUpdate(); } catch (e) {
      // the app's server could not reach GitHub: try from the page (another network stack)
      u = await directRelease().catch(() => null);
      if (!u) throw e;
    }
    if (u.status === 'private' || u.status === 'bad_token') {
      if (interactive) {
        out(u.status === 'private'
          ? 'Не удалось найти релизы приложения на GitHub. Откройте страницу релизов:'
          : 'GitHub не принял токен (истёк или нет доступа к репозиторию). Обновите его в «Настройки → Приложение».',
        [h('a', { class: 'btn primary', href: '#/settings' }, 'Открыть настройки'), h('a', { class: 'btn', href: RELEASES_PAGE }, 'Релизы на GitHub')]);
      }
      return null;
    }
    save({ lastAppCheck: Date.now() });
    if (!u.version) { if (interactive) out('Релизов приложения пока нет.'); return null; }
    const ver = u.version;
    if (newer(ver, APP_VERSION) && (u.asset_id || u.public_url)) {
      const size = u.size ? ` (${(u.size / 1048576).toFixed(1).replace('.', ',')} МБ)` : '';
      const canSelf = !!(window.VNShelfNative && window.VNShelfNative.startUpdate);
      // the APK comes through the local server, which adds the token for a private repository
      // through the local server (adds a token if needed) or, without an asset id, straight from GitHub
      const local = u.asset_id ? `${location.origin}/api/app-update/apk?id=${u.asset_id}&repo=${u.repo || 'public'}` : u.public_url;
      out(`⬆️ Доступна версия ${ver} (у вас ${APP_VERSION}).`, [
        canSelf
          ? h('button', { class: 'btn primary', onclick: () => selfUpdate(local, ver, out) }, `Обновить${size}`)
          : h('a', { class: 'btn primary', href: u.public_url || RELEASES_PAGE }, `Скачать APK${size}`),
        h('a', { class: 'btn', href: u.html_url || RELEASES_PAGE }, 'Что нового')]);
    } else if (interactive) {
      if (show) out(`✅ У вас последняя версия ${APP_VERSION}.`); else toast(`Установлена последняя версия (${APP_VERSION})`);
    }
    return u;
  } catch (e) {
    if (interactive) { if (show) out(`⚠️ ${e.message}`, [h('a', { class: 'btn', href: RELEASES_PAGE }, 'Скачать вручную')]); else toast(e.message); }
    return null;
  }
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
      out(h('div', { class: 'job compact' },
        h('div', { class: 'job-head' }, h('b', {}, pct !== null ? `${pct}%` : '…'), h('span', {}, ` Загружаю версию ${ver} · ${known ? `${mb(st.done)} из ${mb(st.total)} МБ` : `скачано ${mb(st.done)} МБ`}`)),
        h('div', { class: `bar ${pct === null ? 'indet' : ''}` }, h('i', { style: pct === null ? {} : { width: `${pct}%` } }))));
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

applyTheme();
buildNav();
installTagTips();
window.addEventListener('hashchange', route);
route();
setTimeout(() => checkUpdates(), 1500);
// a job may already be running (e.g. the app was reopened during a download)
api.status().then(st => { if (st.ready && st.import?.running) watchJob('✅ Готово.'); }).catch(() => {});
setTimeout(() => checkAppUpdate(), 4000);

if ('serviceWorker' in navigator && location.protocol !== 'file:') {
  navigator.serviceWorker.register('sw.js').catch(() => {});
}
