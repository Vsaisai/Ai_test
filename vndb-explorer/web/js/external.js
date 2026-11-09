// Games from external catalogs (F95Zone): metadata only, every game leads to its original page.
// Shown only in the "show everything" content mode with the catalog switched on in Settings.
import { api } from './api.js';
import { settings, save } from './store.js';
import { h, spinner, errorBox, toast, debounce, jobPanel } from './ui.js';

export const extOn = () => !!settings().adultTags && !!settings().f95;

const STATUS = { ongoing: 'В разработке', completed: 'Завершена', onhold: 'Заморожена', abandoned: 'Брошена' };
const SORTS = { updated: 'Недавно обновлённые', rating: 'Рейтинг', likes: 'Популярные', title: 'По алфавиту' };
const NOTE = 'Каталог сообщества F95Zone: только описания и ссылки на исходные страницы, без файлов. ' +
  'Наличие игры там не означает, что её распространение законно — поддерживайте разработчиков на их официальных страницах.';

function mount(root, ...children) {
  root.replaceChildren(...children.flat(Infinity).filter(c => c !== null && c !== undefined && c !== false));
}

const fmtDate = d => (d ? d.split('-').reverse().join('.') : '');

/** Cover straight from the site (never stored by the app); blurred unless covers are shown openly. */
function extCover(url, alt, cls = 'cover') {
  if (!url) return h('div', { class: `${cls} nocover` }, h('span', {}, 'нет обложки'));
  const box = h('div', { class: cls }, h('img', { src: url, alt, loading: 'lazy', decoding: 'async', referrerpolicy: 'no-referrer' }));
  if (settings().blur !== 'off') {
    box.classList.add('blurred');
    box.append(h('button', {
      class: 'reveal', type: 'button', 'aria-label': 'Показать изображение',
      onclick: e => { e.preventDefault(); e.stopPropagation(); box.classList.remove('blurred'); e.currentTarget.remove(); },
    }, 'Показать'));
  }
  return box;
}

/** Card: merged games open their VNDB page, the rest their own page. */
export function extCard(g) {
  const href = g.vndb_id ? `#/vn/${g.vndb_id}` : `#/f95/${g.ext_id}`;
  const meta = [g.version, STATUS[g.status], fmtDate(g.updated)].filter(Boolean).join(' · ');
  return h('a', { class: 'card', href, title: g.developer ? `${g.title}\n${g.developer}` : g.title },
    h('div', { class: 'cover-wrap' }, extCover(g.cover, g.title),
      h('div', { class: 'badges' }, g.vndb_id ? h('span', { class: 'badge full', title: 'Эта игра есть в каталоге VNDB' }, 'VNDB') : null)),
    h('div', { class: 'card-title' }, g.title),
    h('div', { class: 'card-meta' }, h('span', {}, g.developer || '')),
    h('div', { class: 'card-meta' }, h('span', { class: 'muted' }, meta)));
}

/** Start a catalog update and show its progress in `box`; `done` runs when it finishes. */
function runRefresh(box, full, done) {
  api.extRefresh(full).then(r => {
    if (!r.started) { toast('Уже идёт другое обновление — попробуйте позже'); return; }
    const poll = async () => {
      const st = await api.status().catch(() => null);
      const job = st && st.import;
      if (job && job.kind === 'external') box.replaceChildren(jobPanel(job));
      if (job && job.running) { setTimeout(poll, 1000); return; }
      done && done(job);
    };
    poll();
  }).catch(e => box.replaceChildren(errorBox(e)));
}

function statusLine(s) {
  if (!s || !s.games) return 'Каталог ещё не загружен.';
  const when = s.listed_at ? new Date(Number(s.listed_at) * 1000).toLocaleString('ru') : '—';
  return `Игр: ${s.games.toLocaleString('ru')}, из них есть в VNDB: ${s.in_vndb.toLocaleString('ru')}. Обновлено: ${when}.` +
    (s.hidden_by_policy ? ` Скрыто правилами приложения: ${s.hidden_by_policy}.` : '');
}

/** Settings block (shown in "Дополнительно"). */
export function extSettings() {
  const s = settings();
  const info = h('p', { class: 'muted small' }, s.adultTags ? 'Загрузка…' : 'Доступно, когда в «Что показывать» выбрано «Показывать всё».');
  const job = h('div', {});
  const err = h('p', { class: 'small' });
  const load = () => api.extStatus().then(r => {
    const f = r.f95zone;
    info.textContent = statusLine(f);
    err.textContent = f && f.last_error ? `⚠️ Последнее обновление остановлено: ${f.last_error}` : '';
  }).catch(() => { info.textContent = 'Статус недоступен.'; });
  const toggle = h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: !!s.f95, disabled: !s.adultTags, onchange: e => { save({ f95: e.target.checked }); toast('Сохранено'); } }),
    h('span', {}, 'Показывать западные игры из каталога F95Zone'));
  if (s.adultTags) load();
  return h('div', {},
    h('h3', {}, 'F95Zone'),
    toggle,
    h('p', { class: 'muted small' }, NOTE),
    info, err,
    h('div', { class: 'toolbar' },
      h('button', { class: 'btn', disabled: !s.adultTags, onclick: () => runRefresh(job, false, load) }, 'Обновить'),
      h('button', { class: 'btn', disabled: !s.adultTags, title: 'Читает весь каталог: несколько минут', onclick: () => runRefresh(job, true, load) }, 'Загрузить полностью')),
    h('p', { class: 'muted small' }, 'Приложение читает только публичную ленту обновлений и страницы игр, не чаще одного запроса в 2 секунды. ' +
      'Если сайт просит пройти проверку на робота или войти, обновление останавливается — обходить это приложение не будет.'),
    job);
}

/** Row for the home page: the latest updated games. */
export function extHomeSection() {
  if (!extOn()) return null;
  const row = h('div', { class: 'shelf-row' }, spinner());
  const sec = h('section', { class: 'shelf-section', id: 'sec-f95' },
    h('h2', {}, 'Западные игры (F95Zone)'),
    h('div', { class: 'shelf' },
      h('a', { class: 'shelf-head', href: '#/f95' }, h('span', { class: 'shelf-title' }, 'Недавно обновлённые'), h('span', { class: 'shelf-all' }, 'все →')),
      row));
  api.extSearch({ sort: 'updated', per_page: 16 }).then(r => {
    if (!r.total) { row.replaceChildren(h('p', { class: 'muted small' }, 'Каталог пуст. ', h('a', { href: '#/f95' }, 'Загрузить'))); return; }
    row.replaceChildren(...r.items.map(extCard));
  }).catch(() => sec.remove());
  return sec;
}

function disabledNote(root) {
  mount(root, h('div', { class: 'panel center' },
    h('h2', {}, 'Каталог F95Zone выключен'),
    h('p', {}, 'Включите его в настройках: «Что показывать» → «Показывать всё», затем «Дополнительно» → F95Zone.'),
    h('a', { class: 'btn primary', href: '#/settings' }, 'Открыть настройки')));
}

export async function viewExtList(root, params) {
  if (!extOn()) return disabledNote(root);
  document.title = 'F95Zone — VN Shelf';
  const st = {
    q: params.get('q') || '', status: params.get('status') || '', engine: params.get('engine') || '',
    sort: params.get('sort') || 'updated', tags: (params.get('tags') || '').split('|').filter(Boolean),
    only_new: params.get('new') === '1', page: Math.max(1, Number(params.get('page')) || 1),
  };
  const write = () => {
    const p = new URLSearchParams();
    if (st.q) p.set('q', st.q);
    if (st.status) p.set('status', st.status);
    if (st.engine) p.set('engine', st.engine);
    if (st.sort !== 'updated') p.set('sort', st.sort);
    if (st.tags.length) p.set('tags', st.tags.join('|'));
    if (st.only_new) p.set('new', '1');
    if (st.page > 1) p.set('page', st.page);
    const qs = p.toString();
    history.replaceState(null, '', `#/f95${qs ? `?${qs}` : ''}`);
  };
  const grid = h('div', { class: 'grid' }, spinner());
  const count = h('p', { class: 'muted small' });
  const pager = h('div', { class: 'toolbar' });
  const job = h('div', {});
  const load = async () => {
    write();
    grid.replaceChildren(spinner());
    try {
      const r = await api.extSearch({ q: st.q, status: st.status || null, engine: st.engine || null, sort: st.sort, tags: st.tags, only_new: st.only_new, page: st.page, per_page: 48 });
      count.textContent = `Найдено: ${r.total.toLocaleString('ru')}`;
      if (!r.total) {
        const empty = !st.q && !st.status && !st.engine && !st.tags.length && !st.only_new;
        grid.replaceChildren(h('div', { class: 'panel center' },
          h('p', {}, empty ? 'Каталог ещё не загружен.' : 'Ничего не нашлось.'),
          empty ? h('button', { class: 'btn primary', onclick: () => runRefresh(job, true, () => load()) }, 'Загрузить каталог') : null));
        pager.replaceChildren();
        return;
      }
      grid.replaceChildren(...r.items.map(extCard));
      const pages = Math.ceil(r.total / r.per_page);
      pager.replaceChildren(
        st.page > 1 ? h('button', { class: 'btn', onclick: () => { st.page--; load(); window.scrollTo(0, 0); } }, '← Назад') : null,
        h('span', { class: 'muted small' }, ` ${st.page} из ${pages} `),
        st.page < pages ? h('button', { class: 'btn', onclick: () => { st.page++; load(); window.scrollTo(0, 0); } }, 'Дальше →') : null);
    } catch (e) {
      grid.replaceChildren(errorBox(e, load));
    }
  };
  const reset = () => { st.page = 1; load(); };
  const q = h('input', { type: 'search', placeholder: 'Название или разработчик', value: st.q, oninput: debounce(e => { st.q = e.target.value.trim(); reset(); }, 300) });
  const sel = (key, opts) => h('select', { onchange: e => { st[key] = e.target.value; reset(); } },
    opts.map(([v, l]) => h('option', { value: v, selected: st[key] === v }, l)));
  const tagBox = h('div', { class: 'choice' });
  const paintTags = all => tagBox.replaceChildren(...all.slice(0, 40).map(({ tag, games }) => h('button', {
    type: 'button', class: st.tags.includes(tag) ? 'on' : '',
    onclick: e => { st.tags = st.tags.includes(tag) ? st.tags.filter(t => t !== tag) : [...st.tags, tag]; e.currentTarget.classList.toggle('on'); reset(); },
  }, tag, h('small', { class: 'muted' }, ` ${games}`))));
  api.extTags().then(r => paintTags(r.tags)).catch(() => {});
  mount(root,
    h('div', { class: 'page-head' }, h('h1', {}, 'Западные игры (F95Zone)')),
    h('p', { class: 'muted small' }, NOTE),
    h('div', { class: 'panel form ext-filters' },
      q,
      h('div', { class: 'toolbar' },
        sel('sort', Object.entries(SORTS)),
        sel('status', [['', 'Любой статус'], ...Object.entries(STATUS)]),
        sel('engine', [['', 'Любой движок'], ...["Ren'Py", 'Unity', 'RPGM', 'HTML', 'Unreal Engine', 'Godot', 'Others'].map(e => [e, e])])),
      h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: st.only_new, onchange: e => { st.only_new = e.target.checked; reset(); } }), h('span', {}, 'Только те, которых нет в VNDB')),
      h('details', {}, h('summary', {}, 'Теги'), tagBox)),
    job, count, grid, pager);
  load();
}

export async function viewExtGame(root, id) {
  if (!extOn()) return disabledNote(root);
  mount(root, spinner('Читаю страницу игры…'));
  let g;
  try {
    g = await api.extGame(id);
  } catch (e) {
    mount(root, errorBox(e, () => viewExtGame(root, id)));
    return;
  }
  document.title = `${g.title} — VN Shelf`;
  const facts = [
    ['Разработчик', g.developer], ['Версия', g.version], ['Статус', STATUS[g.status]], ['Движок', g.engine],
    ['Обновлено', fmtDate(g.updated)], ['Рейтинг на F95Zone', g.rating ? `${g.rating.toFixed(1)} из 5` : ''],
  ].filter(([, v]) => v);
  mount(root,
    h('div', { class: 'page-head' }, h('a', { class: 'back', href: '#/f95' }, '← Все игры F95Zone')),
    h('div', { class: 'vn-hero' },
      extCover(g.cover, g.title, 'cover big'),
      h('div', { class: 'vn-head' },
        h('h1', {}, g.title),
        h('dl', { class: 'meta' }, facts.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])),
        g.vndb_id ? h('p', {}, h('a', { class: 'btn primary', href: `#/vn/${g.vndb_id}` }, `Открыть в каталоге VNDB${g.vndb_title ? `: ${g.vndb_title}` : ''}`)) : null)),
    h('section', { class: 'panel where' },
      h('h2', {}, 'Где взять'),
      g.links.length
        ? h('div', { class: 'where-group' }, h('h3', {}, 'Официальные страницы разработчика'),
          h('div', { class: 'toolbar' }, g.links.map(l => h('a', { class: 'btn', href: l.url, target: '_blank', rel: 'noopener noreferrer' }, l.label || l.site, ' ↗'))))
        : h('p', { class: 'muted small' }, g.details_at ? 'Официальных страниц на странице игры не нашлось.' : 'Официальные страницы появятся, когда приложение прочитает страницу игры.'),
      h('div', { class: 'where-group community' }, h('h3', {}, 'Каталог сообщества'),
        h('a', { href: g.page_url, target: '_blank', rel: 'noopener noreferrer' }, 'Страница игры на F95Zone ↗')),
      h('p', { class: 'muted small' }, NOTE)),
    g.description ? h('section', {}, h('h2', {}, 'Описание'), h('p', { class: 'desc', style: 'white-space: pre-line' }, g.description),
      h('p', { class: 'muted small' }, 'Отрывок со страницы игры на F95Zone (на языке оригинала).')) : null,
    g.tags.length ? h('section', {}, h('h2', {}, 'Теги'), h('div', { class: 'pills' }, g.tags.map(t => h('a', { class: 'pill', href: `#/f95?tags=${encodeURIComponent(t)}` }, t)))) : null,
    g.details_error ? h('p', { class: 'muted small' }, `⚠️ Страницу игры сейчас прочитать не удалось (${g.details_error}). Показаны сохранённые данные.`) : null);
}

/** Links on a VNDB page to the same game in external catalogs. */
export function extVnBlock(d) {
  if (!extOn() || !d.external || !d.external.length) return null;
  return h('div', { class: 'where-group community' },
    h('h3', {}, 'Каталог сообщества F95Zone'),
    d.external.map(x => h('div', { class: 'source' },
      h('a', { href: `#/f95/${x.ext_id}` }, `Карточка F95Zone${x.version ? ` · ${x.version}` : ''}${x.status && STATUS[x.status] ? ` · ${STATUS[x.status]}` : ''}`),
      ' · ', h('a', { href: x.url, target: '_blank', rel: 'noopener noreferrer' }, 'страница на сайте ↗'))));
}
