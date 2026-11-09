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
  const href = g.vndb_id && !(g.match_kind === 'title' && !g.match_manual) ? `#/vn/${g.vndb_id}` : `#/f95/${g.ext_id}`;
  const meta = [g.version, STATUS[g.status], fmtDate(g.updated)].filter(Boolean).join(' · ');
  return h('a', { class: 'card', href, title: g.developer ? `${g.title}\n${g.developer}` : g.title },
    h('div', { class: 'cover-wrap' }, extCover(g.cover, g.title),
      h('div', { class: 'badges' }, g.vndb_id ? h('span', { class: `badge ${g.match_kind === 'title' && !g.match_manual ? '' : 'full'}`, title: g.match_kind === 'title' && !g.match_manual ? 'Найдена в VNDB только по названию — проверьте' : 'Эта игра есть в каталоге VNDB' }, g.match_kind === 'title' && !g.match_manual ? 'VNDB?' : 'VNDB') : null)),
    h('div', { class: 'card-title' }, g.title),
    h('div', { class: 'card-meta' }, h('span', {}, g.developer || '')),
    h('div', { class: 'card-meta' }, h('span', { class: 'muted' }, meta)));
}

/** Start a catalog update and show its progress in `box`; `done` runs when it finishes. */
function runRefresh(box, full, done) {
  runJob(box, () => api.extRefresh(full), done);
}

const EXT_JOBS = ['external', 'external-audit', 'external-check'];

function runJob(box, start, done) {
  start().then(r => {
    if (!r.started) { toast('Уже идёт другое обновление — попробуйте позже'); return; }
    const poll = async () => {
      const st = await api.status().catch(() => null);
      const job = st && st.import;
      if (job && EXT_JOBS.includes(job.kind)) box.replaceChildren(jobPanel(job));
      if (job && job.running) { setTimeout(poll, 1000); return; }
      done && done(job);
    };
    poll();
  }).catch(e => box.replaceChildren(errorBox(e)));
}

const when = t => (t ? new Date(Number(t) * 1000).toLocaleString('ru') : '—');
const n = x => (x || 0).toLocaleString('ru');

function statusLine(s) {
  if (!s || !s.stored) return 'Каталог ещё не загружен.';
  return `Показывается игр: ${n(s.games)}, из них есть в VNDB: ${n(s.in_vndb)}` +
    (s.manual_matches ? ` (исправлено вручную: ${n(s.manual_matches)})` : '') + `. Всего записей: ${n(s.stored)}. Обновлено: ${when(s.listed_at)}.`;
}

/** Is the catalog complete? Only a finished comparison with the forum can say so. */
function completeness(s) {
  const a = s.last_audit;
  if (!s.feed_complete_at) return ['warn', 'Полнота не подтверждена: лента ещё не прочитана целиком. Нажмите «Загрузить всю ленту».'];
  if (!a) return ['warn', `Лента прочитана целиком (${when(s.feed_complete_at)}), но полнота не подтверждена: нужна «Сверка с форумом».`];
  if (!a.complete || a.stopped) return ['warn', `Сверка с форумом не завершена (${a.stopped || `не прочитаны страницы: ${a.pages_failed.join(', ')}`}). Полнота не подтверждена.`];
  if (s.needs_check) return ['warn', `Сверка ${when(a.finished_at)}: в форуме ${n(a.index_threads)} тем, все записаны в каталог, но у ${n(s.needs_check)} ещё не прочитаны страницы (нужны теги). Запустите сверку ещё раз.`];
  return ['ok', `✓ Полнота подтверждена сверкой ${when(a.finished_at)}: все ${n(a.index_threads)} тем форума Games есть в каталоге` +
    (a.missing_added ? ` (${n(a.missing_added)} из них лента не показывала — добавлены при сверке)` : '') + '.'];
}

function reportBox(s) {
  const r = s.last_report;
  if (!r) return null;
  const skipped = (r.unchanged || 0) + (r.hidden_by_policy || 0);
  const rows = [
    ['Обнаружено релизов', n(r.discovered)],
    ['Новых записей', n(r.new)],
    ['Обновлённых записей', n(r.updated)],
    ['Пропущено', `${n(skipped)} (без изменений ${n(r.unchanged)}, скрыто правилами ${n(r.hidden_by_policy)})`],
    ['Ошибок', `${n(r.errors)}${r.pages_failed && r.pages_failed.length ? ` (страницы: ${r.pages_failed.join(', ')})` : ''}`],
    ['Дубликатов', `${n(r.duplicates)} (одинаковые название и разработчик)${r.repeats ? `, повторов в ленте: ${n(r.repeats)}` : ''}`],
    ['Объединено с VNDB', r.in_vndb === null || r.in_vndb === undefined ? '—' : n(r.in_vndb)],
    ['Страниц ленты', `${n(r.pages_read)} прочитано${r.pages_total ? ` из ${n(r.pages_total)}` : ''}${r.resumed_from ? `, продолжено со страницы ${r.resumed_from}` : ''}`],
  ];
  return h('div', { class: 'ext-report' },
    h('b', {}, `Последний импорт (${r.mode === 'full' ? 'вся лента' : 'обновление'}, ${when(r.finished_at)})${r.complete ? '' : r.mode === 'full' ? ' — не завершён' : ''}`),
    h('dl', { class: 'meta' }, rows.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])),
    r.stopped ? h('p', { class: 'small' }, `⚠️ Остановлено: ${r.stopped}`) : null);
}

function hiddenNote(s) {
  if (!s.hidden_by_policy) return null;
  const reasons = Object.entries(s.hidden_reasons || {}).map(([k, v]) => `${k}: ${n(v)}`).join(', ');
  return h('p', { class: 'muted small' }, `Скрыто правилами приложения: ${n(s.hidden_by_policy)} (${reasons}). ` +
    'Эти записи сохранены и учтены в отчётах, но не показываются нигде в приложении.');
}

function checkBox(c) {
  if (!c) return null;
  return h('details', { class: 'ext-check' },
    h('summary', {}, `Проверка подключения ${when(c.at)}: ${c.ok ? '✓ всё в порядке' : c.stopped ? `остановлена — ${c.stopped}` : 'есть проблемы'}`),
    h('ul', { class: 'small' }, c.steps.map(st => h('li', {}, `${st.ok ? '✓' : '✗'} ${st.name}: ${st.details.join('; ')}`))));
}

/** Settings block (shown in "Дополнительно"). */
export function extSettings() {
  const s = settings();
  const box = h('div', {}, h('p', { class: 'muted small' }, s.adultTags ? 'Загрузка…' : 'Доступно, когда в «Что показывать» выбрано «Показывать всё».'));
  const job = h('div', {});
  const load = () => api.extStatus().then(r => {
    const f = r.f95zone;
    const [kind, text] = completeness(f);
    mount(box,
      h('p', { class: 'muted small' }, statusLine(f)),
      f.stored ? h('p', { class: `small ${kind === 'ok' ? 'ok-line' : ''}` }, kind === 'ok' ? text : `⚠️ ${text}`) : null,
      f.cooldown_left ? h('p', { class: 'small' }, `⏸ Сайт попросил паузу (слишком много запросов). Можно нажимать кнопки — работа начнётся сама через ~${Math.ceil(f.cooldown_left / 60)} мин.`) : null,
      f.last_error ? h('p', { class: 'small' }, `⚠️ Последнее обновление остановлено: ${f.last_error}${/подождать/.test(f.last_error) ? '. Нажмите «Загрузить всю ленту» позже — продолжится с того же места.' : ''}`) : null,
      reportBox(f), hiddenNote(f), checkBox(f.last_check));
  }).catch(() => mount(box, h('p', { class: 'muted small' }, 'Статус недоступен.')));
  const toggle = h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: !!s.f95, disabled: !s.adultTags, onchange: e => { save({ f95: e.target.checked }); toast('Сохранено'); } }),
    h('span', {}, 'Показывать западные игры из каталога F95Zone'));
  if (s.adultTags) load();
  const btn = (text, title, start) => h('button', { class: 'btn', disabled: !s.adultTags, title, onclick: () => runJob(job, start, load) }, text);
  return h('div', {},
    h('h3', {}, 'F95Zone'),
    toggle,
    h('p', { class: 'muted small' }, NOTE),
    box,
    h('div', { class: 'toolbar' },
      btn('Обновить', 'Новые страницы ленты, пока две подряд не принесут ничего нового', () => api.extRefresh(false)),
      btn('Загрузить всю ленту', 'Все страницы ленты; прерванная загрузка продолжится с места остановки', () => api.extRefresh(true)),
      btn('Сверить с форумом', 'Все страницы списка тем форума Games: находит игры, которых нет в ленте', () => api.extAudit()),
      btn('Проверить подключение', 'Около 7 запросов: лента, форум и три страницы игр', () => api.extCheck())),
    h('p', { class: 'muted small' }, 'Приложение читает только публичную ленту обновлений, список тем форума Games и страницы игр, не чаще одного запроса в 3 секунды. ' +
      'Если сайт говорит «слишком много запросов», приложение ждёт столько, сколько он просит (обычно 15 минут), и продолжает медленнее. ' +
      'Если сайт просит пройти проверку на робота или войти, работа останавливается — обходить это приложение не будет.'),
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
    only_new: params.get('new') === '1', matches: params.get('match') || '', page: Math.max(1, Number(params.get('page')) || 1),
  };
  const write = () => {
    const p = new URLSearchParams();
    if (st.q) p.set('q', st.q);
    if (st.status) p.set('status', st.status);
    if (st.engine) p.set('engine', st.engine);
    if (st.sort !== 'updated') p.set('sort', st.sort);
    if (st.tags.length) p.set('tags', st.tags.join('|'));
    if (st.only_new) p.set('new', '1');
    if (st.matches) p.set('match', st.matches);
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
      const r = await api.extSearch({ q: st.q, status: st.status || null, engine: st.engine || null, sort: st.sort, tags: st.tags, only_new: st.only_new, matches: st.matches || null, page: st.page, per_page: 48 });
      count.textContent = `Найдено: ${r.total.toLocaleString('ru')}`;
      if (!r.total) {
        const empty = !st.q && !st.status && !st.engine && !st.tags.length && !st.only_new && !st.matches;
        grid.replaceChildren(h('div', { class: 'panel center' },
          h('p', {}, empty ? 'Каталог ещё не загружен.' : 'Ничего не нашлось.'),
          empty ? h('button', { class: 'btn primary', onclick: () => runRefresh(job, true, () => load()) }, 'Загрузить каталог') : null));
        pager.replaceChildren();
        return;
      }
      grid.replaceChildren(...r.items.map(extCard));
      const pages = Math.ceil(r.total / r.per_page);
      mount(pager, pages > 1 ? [
        st.page > 1 ? h('button', { class: 'btn', onclick: () => { st.page--; load(); window.scrollTo(0, 0); } }, '← Назад') : null,
        h('span', { class: 'muted small' }, ` ${st.page} из ${pages} `),
        st.page < pages ? h('button', { class: 'btn', onclick: () => { st.page++; load(); window.scrollTo(0, 0); } }, 'Дальше →') : null] : null);
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
        sel('engine', [['', 'Любой движок'], ...["Ren'Py", 'Unity', 'RPGM', 'HTML', 'Unreal Engine', 'Godot', 'Others'].map(e => [e, e])]),
        sel('matches', [['', 'Совпадения с VNDB: все'], ['weak', 'Найдены только по названию — проверить'], ['manual', 'Исправлены вручную'], ['none', 'Нет в VNDB'], ['any', 'Есть в VNDB']])),
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
    matchBox(g, () => viewExtGame(root, id)),
    h('section', { class: 'panel where' },
      h('h2', {}, 'Где взять'),
      g.links.length
        ? h('div', { class: 'where-group' }, h('h3', {}, 'Официальные страницы разработчика'),
          h('div', { class: 'toolbar' }, g.links.map(l => h('a', { class: 'btn', href: l.url, target: '_blank', rel: 'noopener noreferrer' }, l.label || l.site, ' ↗'))))
        : h('p', { class: 'muted small' }, g.details_at ? 'Официальных страниц на странице игры не нашлось.' : 'Официальные страницы появятся, когда приложение прочитает страницу игры.'),
      h('div', { class: 'where-group community' }, h('h3', {}, 'Каталог сообщества'),
        h('a', { href: g.page_url, target: '_blank', rel: 'noopener noreferrer' }, 'Страница игры на F95Zone ↗')),
      h('p', { class: 'muted small' }, NOTE)),
    g.description ? descriptionBlock(g) : null,
    g.tags.length ? h('section', {}, h('h2', {}, 'Теги'), h('div', { class: 'pills' }, g.tags.map(t => h('a', { class: 'pill', href: `#/f95?tags=${encodeURIComponent(t)}` }, t)))) : null,
    g.details_error ? h('p', { class: 'muted small' }, `⚠️ Страницу игры сейчас прочитать не удалось (${g.details_error}). Показаны сохранённые данные.`) : null);
}

const MATCH_KIND = {
  link: 'по той же странице игры (Steam, itch.io, DLsite)',
  'title+link': 'по названию и странице разработчика',
  'title+developer': 'по названию и разработчику',
  title: 'только по названию — стоит проверить',
  manual: 'указано вручную',
};

/** VNDB match with manual correction. */
function matchBox(g, reload) {
  const msg = h('p', { class: 'small' });
  const send = async body => {
    try {
      await api.extMatch(g.ext_id, body);
      toast('Сохранено');
      reload();
    } catch (e) { msg.textContent = `⚠️ ${e.message}`; }
  };
  const input = h('input', { type: 'text', placeholder: 'v1234 или ссылка vndb.org/v1234', 'aria-label': 'Номер новеллы VNDB' });
  const current = g.vndb_id
    ? h('p', {}, 'Совпадает с ', h('a', { href: `#/vn/${g.vndb_id}` }, g.vndb_title || `v${g.vndb_id}`), ` (v${g.vndb_id}) — ${MATCH_KIND[g.match_kind] || g.match_kind}.`)
    : h('p', {}, g.match_manual ? 'Отмечено вручную: этой игры нет в VNDB.' : 'В каталоге VNDB не найдена.');
  return h('section', { class: 'panel form ext-match' },
    h('h2', {}, 'Совпадение с VNDB'),
    current,
    h('div', { class: 'toolbar' },
      g.vndb_id && !g.match_manual ? h('button', { class: 'btn', onclick: () => send({ vndb: `v${g.vndb_id}` }) }, 'Верно') : null,
      g.vndb_id ? h('button', { class: 'btn', onclick: () => send({ vndb: null }) }, 'Неверно — отвязать') : null,
      !g.vndb_id && !g.match_manual ? h('button', { class: 'btn', onclick: () => send({ vndb: null }) }, 'Подтвердить: нет в VNDB') : null,
      g.match_manual ? h('button', { class: 'btn', onclick: () => send({ auto: true }) }, 'Вернуть автоподбор') : null),
    h('div', { class: 'toolbar' }, input,
      h('button', { class: 'btn', onclick: () => { if (input.value.trim()) send({ vndb: input.value.trim() }); } }, 'Указать другую новеллу')),
    msg,
    h('p', { class: 'muted small' }, 'Ручное решение не меняется при следующих обновлениях каталога.'));
}

/** The whole public overview of the game, folded when long. */
function descriptionBlock(g) {
  const text = g.description;
  const long = text.length > 1500;
  const body = h('p', { class: 'desc', style: 'white-space: pre-line' }, long ? `${text.slice(0, 1200).replace(/\s+\S*$/, '')}…` : text);
  const more = long ? h('button', { class: 'linkish', onclick: () => { body.textContent = text; more.remove(); } }, 'Показать полностью') : null;
  return h('section', {}, h('h2', {}, 'Описание'), body, more,
    h('p', { class: 'muted small' }, 'Описание со страницы игры на F95Zone, на языке оригинала. Читается, только когда вы открываете игру.'));
}

/** Links on a VNDB page to the same game in external catalogs. */
export function extVnBlock(d) {
  if (!extOn() || !d.external || !d.external.length) return null;
  return h('div', { class: 'where-group community' },
    h('h3', {}, 'Каталог сообщества F95Zone'),
    d.external.map(x => h('div', { class: 'source' },
      h('a', { href: `#/f95/${x.ext_id}` }, `Карточка F95Zone${x.version ? ` · ${x.version}` : ''}${x.status && STATUS[x.status] ? ` · ${STATUS[x.status]}` : ''}`),
      ' · ', h('a', { href: x.url, target: '_blank', rel: 'noopener noreferrer' }, 'страница на сайте ↗'),
      ' · ', h('button', { class: 'linkish small', title: 'Это другая игра: убрать связь', onclick: async e => {
        try { await api.extMatch(x.ext_id, { vndb: null }); toast('Связь убрана'); e.target.closest('.source').remove(); } catch (err) { toast(err.message); }
      } }, 'не та игра'))));
}
