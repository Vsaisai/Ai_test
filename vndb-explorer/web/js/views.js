// All screens of the app.
import { api, img } from './api.js';
import { settings, save, baseRequest, DEFAULTS, IN_APP, APP_VERSION, PLATFORM_GROUPS } from './store.js';
import {
  h, card, cover, spinner, errorBox, toast, debounce, langBadges, fmtLength,
  LANG_NAMES, PLATFORMS, MARKS, jobPanel, langLegend, tagDescription, richText, ruLabel,
} from './ui.js';
import { CURATED, ADULT, tagLabel, catRu, resolveGroup, hasRu } from './taxonomy.js';
import { openLightbox } from './lightbox.js';

const SORTS = {
  relevance: 'По соответствию', votes: 'Популярные', rating: 'Рейтинг', newest: 'Новые', oldest: 'Старые',
  title: 'По алфавиту', shortest: 'Короткие', longest: 'Длинные',
};

const CJK = ['ja', 'zh', 'zh-Hans', 'zh-Hant', 'ko'];

/** replaceChildren that ignores null/false/undefined (they would otherwise become text). */
function mount(root, ...children) {
  root.replaceChildren(...children.flat(Infinity).filter(c => c !== null && c !== undefined && c !== false));
}

const adultOn = () => !!settings().adultTags;

// ---------- tags helpers ----------

function tagPath(tags, id) {
  const out = [];
  let cur = tags.byId.get(id);
  const seen = new Set();
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    out.unshift(cur);
    cur = cur.parents.length ? tags.byId.get(cur.parents[0]) : null;
  }
  return out;
}

/** The tag and all of its ancestors (through every parent). */
function lineage(tags, id) {
  const out = new Set();
  const stack = [id];
  while (stack.length) {
    const cur = stack.pop();
    if (out.has(cur)) continue;
    out.add(cur);
    const t = tags.byId.get(cur);
    if (t) stack.push(...t.parents);
  }
  return out;
}

function findTags(tags, q, limit = 12, { adultOnly = false } = {}) {
  const s = q.trim().toLowerCase();
  if (!s) return [];
  const adult = adultOn();
  const scored = [];
  for (const t of tags.list) {
    if (adultOnly ? t.cat !== 'ero' : (!adult && t.cat === 'ero')) continue;
    const n = t.name.toLowerCase();
    const ru = tagLabel(t).toLowerCase();
    let score = -1;
    if (n === s || ru === s) score = 100;
    else if (n.startsWith(s) || ru.startsWith(s)) score = 60;
    else if (n.includes(s) || ru.includes(s)) score = 30;
    else if (t.aliases.some(a => a.toLowerCase().includes(s))) score = 20;
    if (score < 0) continue;
    scored.push([score + Math.min(Math.log10(t.vns + 1) * 5, 19), t]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  return scored.slice(0, limit).map(x => x[1]);
}

function pathText(tags, id) {
  return tagPath(tags, id).slice(0, -1).map(x => catRu(x.name)).join(' › ');
}

/** Autocomplete input for tags. `onPick(tag, mode)`; mode is 'must' | 'want' | 'exclude'. */
function tagPicker(tags, onPick, defaultMode = 'want', countOf = t => t.vns) {
  let mode = defaultMode;
  const list = h('div', { class: 'suggest', hidden: true });
  const input = h('input', { type: 'search', placeholder: 'Добавить тег… (например, «петля» или time loop)', autocomplete: 'off' });
  const modes = h('div', { class: 'seg' }, ['must', 'want', 'exclude'].map(m =>
    h('button', {
      type: 'button', class: m === mode ? 'on' : '', 'data-m': m,
      onclick: () => { mode = m; modes.querySelectorAll('button').forEach(b => b.classList.toggle('on', b.dataset.m === m)); input.focus(); },
    }, { must: 'Обязательно', want: 'Желательно', exclude: 'Исключить' }[m])));
  let active = -1;
  const render = () => {
    const found = findTags(tags, input.value);
    list.replaceChildren(...found.map((t, i) => h('button', {
      type: 'button', class: `sug ${i === active ? 'active' : ''}`,
      onmousedown: e => { e.preventDefault(); pick(t); },
    }, h('span', { class: 'sug-name' }, tagLabel(t), tagLabel(t) !== t.name ? h('span', { class: 'sug-en' }, ` ${t.name}`) : null),
      h('span', { class: 'sug-path' }, pathText(tags, t.id)),
      h('span', { class: 'sug-count' }, countOf(t)))));
    list.hidden = !found.length;
    list._found = found;
  };
  const pick = t => { onPick(t, mode); input.value = ''; list.hidden = true; active = -1; };
  input.addEventListener('input', () => { active = -1; render(); });
  input.addEventListener('blur', () => setTimeout(() => { list.hidden = true; }, 120));
  input.addEventListener('keydown', e => {
    const f = list._found || [];
    if (e.key === 'ArrowDown') { active = Math.min(active + 1, f.length - 1); render(); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { active = Math.max(active - 1, 0); render(); e.preventDefault(); }
    else if (e.key === 'Enter') { e.preventDefault(); if (f.length) pick(f[Math.max(active, 0)]); }
  });
  return h('div', { class: 'picker' }, modes, h('div', { class: 'picker-input' }, input, list));
}

// ---------- grid with "load more" ----------

/** Cards link to the VN page with `?hl=` (the tags that were searched) so they can be highlighted. */
function resultGrid(loader, { wantCount = 0, empty = 'Ничего не найдено.', hl = null } = {}) {
  const grid = h('div', { class: 'grid' });
  const info = h('div', { class: 'result-info' });
  const more = h('button', { class: 'btn wide', hidden: true }, 'Показать ещё');
  const wrap = h('div', {}, info, grid, more);
  let offset = 0;
  let total = 0;
  let highlight = hl;
  const load = async () => {
    more.disabled = true;
    const sp = spinner();
    wrap.append(sp);
    try {
      const r = await loader(offset);
      if (!highlight) highlight = [...(r.must || []), ...(r.want || [])].map(t => t.id);
      total = r.total;
      offset += r.items.length;
      grid.append(...r.items.map(it => card(it, { wantCount, hl: highlight })));
      info.textContent = total ? `Найдено: ${total.toLocaleString('ru')}` : empty;
      if (r.warnings && r.warnings.length) info.append(h('div', { class: 'warn' }, r.warnings.join('; ')));
      more.hidden = offset >= total;
    } catch (e) {
      wrap.append(errorBox(e, load));
    } finally {
      sp.remove();
      more.disabled = false;
    }
  };
  more.onclick = load;
  load();
  return wrap;
}

// ---------- home: shelves ----------

export async function viewHome(root) {
  root.append(spinner('Раскладываем по полкам…'));
  let st;
  try {
    st = await api.status();
  } catch (e) {
    mount(root, errorBox(e, () => location.reload()));
    return;
  }
  if (!st.ready) {
    mount(root, emptyDb(st));
    return;
  }
  try {
    const data = await api.shelves(baseRequest(), adultOn(), 16);
    mount(root,
      h('div', { class: 'home-head' },
        h('p', { class: 'muted small' }, `${st.vns.toLocaleString('ru')} новелл · база от ${(st.dump_timestamp || '').slice(0, 10)}`),
        h('div', { class: 'row-actions' },
          h('button', { class: 'btn', onclick: async () => {
            const r = await api.random({ ...baseRequest(), min_votes: 20 });
            if (r.id) location.hash = `#/vn/${r.id}`;
          } }, '🎲 Случайная'))),
      // table of contents: the page is long, jump straight to a section
      h('nav', { class: 'toc', 'aria-label': 'Разделы' }, data.sections.filter(sec => sec.shelves.length).map(sec =>
        h('a', { href: `#sec-${sec.id}`, class: 'pill', onclick: e => {
          e.preventDefault(); document.getElementById(`sec-${sec.id}`)?.scrollIntoView({ behavior: 'smooth' });
        } }, sec.title))),
      data.sections.filter(sec => sec.shelves.length).map(sec => h('section', { class: 'shelf-section', id: `sec-${sec.id}` },
        h('h2', {}, sec.title),
        ...sec.shelves.map(sh => h('div', { class: 'shelf' },
          h('a', { class: 'shelf-head', href: `#/shelf/${sh.id}` },
            h('span', { class: 'shelf-title' }, sh.title),
            h('span', { class: 'shelf-all' }, `все ${sh.total.toLocaleString('ru')} →`)),
          h('div', { class: 'shelf-row' }, sh.items.map(it => card(it))))))),
    );
  } catch (e) {
    mount(root, errorBox(e, () => viewHome(root)));
  }
}

function emptyDb(st) {
  const box = h('div', {});
  const btn = h('button', { class: 'btn primary', onclick: async () => {
    btn.disabled = true;
    await api.startImport();
    poll();
  } }, 'Скачать базу VNDB (~190 МБ)');
  const render = job => box.replaceChildren(job && (job.running || job.error) && job.kind === 'import' ? jobPanel(job) : '');
  const poll = async () => {
    const s = await api.status().catch(() => null);
    if (!s) { setTimeout(poll, 2000); return; }
    render(s.import);
    if (s.ready) { toast('База готова'); location.reload(); return; }
    if (s.import.running) setTimeout(poll, 1000); else btn.disabled = false;
  };
  render(st.import);
  if (st.import?.running) { btn.disabled = true; setTimeout(poll, 500); }
  return h('div', { class: 'panel center' },
    h('h2', {}, 'База ещё не загружена'),
    h('p', {}, 'Приложение работает офлайн с полной копией VNDB: новеллы, теги, релизы и ссылки. ' +
      'Скачайте базу один раз — дальше всё работает без интернета (кроме обложек при первом показе).'),
    IN_APP ? h('p', { class: 'muted' }, 'Загрузка ~190 МБ, лучше по Wi-Fi. Займёт около 150 МБ памяти телефона. Можно свернуть приложение — прогресс будет в уведомлении.') : null,
    btn, box);
}

// ---------- shelf page ----------

export async function viewShelf(root, id, params) {
  const sort = params.get('sort') || '';
  const head = h('div', { class: 'page-head' });
  root.append(head);
  let firstShelf = null;
  const grid = resultGrid(async offset => {
    const r = await api.shelf(id, baseRequest(), sort || null, offset);
    if (!firstShelf) {
      firstShelf = r.shelf;
      head.replaceChildren(
        h('h1', {}, r.shelf.title),
        h('div', { class: 'toolbar' },
          sortSelect(sort || r.shelf.sort, v => { location.hash = `#/shelf/${id}?sort=${v}`; }),
          h('a', { class: 'btn', href: searchHashFromQuery(r.shelf.query) }, 'Уточнить в поиске')));
    }
    return r;
  });
  root.append(grid);
}

function sortSelect(value, onChange) {
  return h('label', { class: 'field inline' }, 'Сортировка ',
    h('select', { onchange: e => onChange(e.target.value) },
      Object.entries(SORTS).map(([k, v]) => h('option', { value: k, selected: k === value }, v))));
}

function searchHashFromQuery(q) {
  const p = new URLSearchParams();
  for (const k of ['must', 'want', 'exclude']) if (q[k]?.length) p.set(k, q[k].join('|'));
  if (q.sort) p.set('sort', q.sort);
  if (q.min_votes) p.set('votes', q.min_votes);
  if (q.length_max) p.set('lmax', q.length_max);
  if (q.length_min) p.set('lmin', q.length_min);
  if (q.freeware) p.set('free', '1');
  return `#/search?${p}`;
}

// ---------- search ----------

function readSearch(params) {
  const list = k => (params.get(k) || '').split('|').filter(Boolean);
  return {
    q: params.get('q') || '',
    must: list('must'), want: list('want'), exclude: list('exclude'),
    sort: params.get('sort') || 'relevance',
    plat: list('plat'),
    lmin: params.get('lmin') || '', lmax: params.get('lmax') || '',
    yfrom: params.get('yfrom') || '', yto: params.get('yto') || '',
    votes: params.get('votes') || '', rating: params.get('rating') || '',
    free: params.get('free') === '1',
    mm: params.get('mm') || '',
    dev: params.get('dev') || '',
  };
}

/** API request of a search state (also used by the tag catalog for its live counts). */
function searchRequest(st) {
  const dev = st.dev || settings().platform || 'any';
  return {
    ...baseRequest(),
    text: st.q || null,
    must: st.must.map(String), want: st.want.map(String), exclude: st.exclude.map(String),
    sort: st.sort,
    platforms: st.plat.length ? st.plat : (PLATFORM_GROUPS[dev] || []),
    length_min: st.lmin ? Number(st.lmin) : null, length_max: st.lmax ? Number(st.lmax) : null,
    year_from: st.yfrom ? Number(st.yfrom) : null, year_to: st.yto ? Number(st.yto) : null,
    min_votes: st.votes ? Number(st.votes) : null, min_rating: st.rating ? Number(st.rating) : null,
    freeware: st.free ? true : null,
    min_match: st.mm !== '' ? Number(st.mm) : null,
  };
}

function writeSearch(st) {
  const p = new URLSearchParams();
  if (st.q) p.set('q', st.q);
  for (const k of ['must', 'want', 'exclude', 'plat']) if (st[k].length) p.set(k, st[k].join('|'));
  if (st.sort && st.sort !== 'relevance') p.set('sort', st.sort);
  for (const k of ['lmin', 'lmax', 'yfrom', 'yto', 'votes', 'rating', 'mm']) if (st[k] !== '' && st[k] !== undefined) p.set(k, st[k]);
  if (st.free) p.set('free', '1');
  if (st.dev) p.set('dev', st.dev);
  return `#/search?${p}`;
}

export async function viewSearch(root, params) {
  root.append(spinner());
  let tags;
  try { tags = await api.tags(); } catch (e) { mount(root, errorBox(e)); return; }
  mount(root);
  const st = readSearch(params);
  // tag refs in the URL may be ids or names: normalise to ids
  for (const k of ['must', 'want', 'exclude']) {
    st[k] = st[k].map(x => {
      const n = Number(String(x).replace(/^g/, ''));
      if (tags.byId.has(n)) return n;
      const f = findTags(tags, x, 1)[0];
      return f ? f.id : null;
    }).filter(x => x !== null);
  }
  const go = () => { location.hash = writeSearch(st); };

  const chips = h('div', { class: 'chips' });
  const renderChips = () => {
    chips.replaceChildren(...['must', 'want', 'exclude'].flatMap(k => st[k].map(id => {
      const t = tags.byId.get(id);
      return h('span', { class: `chip ${k}`, 'data-tag': id, title: t ? `${t.name} — ${pathText(tags, id)}` : '' },
        h('button', {
          type: 'button', class: 'chip-mode', title: 'Сменить: обязательно → желательно → исключить',
          onclick: () => {
            const next = { must: 'want', want: 'exclude', exclude: 'must' }[k];
            st[k] = st[k].filter(x => x !== id); st[next].push(id); go();
          },
        }, { must: '!', want: '~', exclude: '−' }[k]),
        t ? tagLabel(t) : `g${id}`,
        h('button', { type: 'button', class: 'chip-x', title: 'Убрать', onclick: () => { st[k] = st[k].filter(x => x !== id); go(); } }, '×'));
    })));
    if (!chips.children.length) chips.append(h('span', { class: 'muted' }, 'Теги не выбраны. «!» — обязательно, «~» — желательно, «−» — исключить. Нажмите на значок, чтобы сменить.'));
  };
  renderChips();

  const text = h('input', { type: 'search', value: st.q, placeholder: 'Название', enterkeyhint: 'search', 'aria-label': 'Название' });
  text.addEventListener('change', () => { st.q = text.value.trim(); go(); });
  const num = (k, ph, w = '6em') => {
    const i = h('input', { type: 'number', value: st[k], placeholder: ph, style: { width: w }, min: 0 });
    i.addEventListener('change', () => { st[k] = i.value; go(); });
    return i;
  };
  const platSel = h('div', { class: 'checks' }, ['win', 'lin', 'mac', 'and', 'ios', 'web', 'swi', 'ps4', 'psv'].map(p =>
    h('label', { class: 'check' }, h('input', {
      type: 'checkbox', checked: st.plat.includes(p),
      onchange: e => { st.plat = e.target.checked ? [...st.plat, p] : st.plat.filter(x => x !== p); go(); },
    }), PLATFORMS[p])));
  const filters = h('details', { class: 'filters', open: !!(st.plat.length || st.lmin || st.lmax || st.yfrom || st.yto || st.votes || st.rating || st.free || st.mm) },
    h('summary', {}, 'Фильтры'),
    h('div', { class: 'filter-grid' },
      h('div', { class: 'field' }, h('span', {}, 'Длина, минут'), h('div', { class: 'pair' }, num('lmin', 'от'), num('lmax', 'до'))),
      h('div', { class: 'field' }, h('span', {}, 'Год выхода'), h('div', { class: 'pair' }, num('yfrom', 'с'), num('yto', 'по'))),
      h('div', { class: 'field' }, h('span', {}, 'Мин. голосов'), num('votes', '0')),
      h('div', { class: 'field' }, h('span', {}, 'Мин. рейтинг (1–10)'), num('rating', '0')),
      h('div', { class: 'field' }, h('span', {}, 'Мин. желательных тегов'), num('mm', '0')),
      h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: st.free, onchange: e => { st.free = e.target.checked; go(); } }), 'Только бесплатные'),
      h('div', { class: 'field full' }, h('span', {}, 'Платформы'), platSel)),
    h('p', { class: 'muted small' }, 'Языки и спойлеры настраиваются в «Настройках».'));

  const dev = st.dev || settings().platform || 'any';
  const devSeg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Где играть' },
    [['any', 'Везде'], ['pc', '💻 ПК'], ['phone', '📱 Телефон']].map(([v, l]) => h('button', {
      type: 'button', class: v === dev ? 'on' : '', onclick: () => { st.dev = v; st.plat = []; go(); },
    }, l)));
  let counts = null; // tag counts over the current results
  root.append(
    h('div', { class: 'page-head' }, h('h1', {}, 'Поиск')),
    h('div', { class: 'panel search-panel' },
      text,
      h('div', { class: 'dev-line' }, h('span', { class: 'muted small' }, 'Где играть:'), devSeg),
      tagPicker(tags, (t, mode) => {
        for (const k of ['must', 'want', 'exclude']) st[k] = st[k].filter(x => x !== t.id);
        st[mode].push(t.id);
        go();
      }, 'want', t => (counts ? (counts[t.id] || 0) : t.vns)),
      chips,
      filters,
      h('div', { class: 'toolbar' },
        sortSelect(st.sort, v => { st.sort = v; go(); }),
        h('a', { class: 'btn', href: `#/tags?${new URLSearchParams(writeSearch(st).split('?')[1] || '')}` }, '🏷️ Каталог тегов'),
        h('button', { class: 'btn', onclick: () => { location.hash = '#/search'; } }, 'Сбросить'))),
  );
  if (!st.must.length && !st.want.length && !st.exclude.length && !st.q && !st.plat.length && !st.free && !st.lmax && !st.lmin && !st.dev) {
    root.append(h('p', { class: 'muted center' }, 'Введите название или добавьте теги — по одному или из «Каталога тегов».'));
    return;
  }
  const req = searchRequest(st);
  root.append(resultGrid(async offset => {
    const r = await api.search({ ...req, offset, limit: 48, tag_counts: offset === 0 });
    if (r.tag_counts) counts = r.tag_counts;
    return r;
  }, { wantCount: st.want.length, hl: [...st.must, ...st.want] }));
}

// ---------- tag catalog ----------

// Tags picked in the catalog (kept while the app is open): id -> 'want' | 'must' | 'exclude'
const picked = new Map();
const NEXT_MODE = { undefined: 'want', want: 'must', must: 'exclude', exclude: undefined };
const MODE_ICON = { want: '~', must: '!', exclude: '−' };
const MODE_NAME = { want: 'желательно', must: 'обязательно', exclude: 'исключить' };

// Counts of the catalog chips under the current selection (required + excluded tags and settings).
let lastCounts = null;
let lastTotal = null;
let countsSeq = 0;
let catalogCarry = new URLSearchParams(); // the search settings the catalog was opened with

function paintCounts() {
  if (!lastCounts) return;
  document.querySelectorAll('.tchip[data-tag]').forEach(el => {
    const n = lastCounts[el.dataset.tag] || 0;
    const c = el.querySelector('.tchip-count');
    if (c) c.textContent = n;
    el.classList.toggle('zero', n === 0 && !el.dataset.mode);
  });
}

const refreshCounts = debounce(async bar => {
  const seq = ++countsSeq;
  const st = { must: [], exclude: [] };
  for (const [id, m] of picked) if (m !== 'want') st[m].push(String(id));
  try {
    // the search's title, platform and filters count too, so "will find N" matches the search
    const base = searchRequest(readSearch(catalogCarry));
    const r = await api.search({ ...base, ...st, want: [], min_match: null, limit: 1, tag_counts: true });
    if (seq !== countsSeq) return;
    lastCounts = r.tag_counts || {};
    lastTotal = r.total;
    paintCounts();
    bar && bar._render();
  } catch { /* counts are a nicety */ }
}, 200);

/** Search URL from the catalog selection plus the search's other settings. */
function searchFromPicked(carry) {
  const p = new URLSearchParams(carry);
  const st = { must: [], want: [], exclude: [] };
  for (const [id, m] of picked) st[m].push(id);
  for (const k of ['must', 'want', 'exclude']) if (st[k].length) p.set(k, st[k].join('|')); else p.delete(k);
  return `#/search?${p}`;
}

function pickBar(carry = new URLSearchParams()) {
  const bar = h('div', { class: 'pickbar', hidden: true });
  const render = () => {
    const count = { want: 0, must: 0, exclude: 0 };
    for (const m of picked.values()) count[m]++;
    bar.hidden = picked.size === 0;
    bar.replaceChildren(
      h('span', { class: 'pickbar-sum' }, `Тегов: ${picked.size}`,
        h('span', { class: 'muted small' }, ` (${Object.entries(count).filter(([, n]) => n).map(([m, n]) => `${MODE_ICON[m]}${n}`).join(' ')})`),
        lastTotal !== null ? h('span', { class: 'pickbar-found' }, `найдётся ${lastTotal.toLocaleString('ru')}`) : null),
      h('button', { class: 'btn', type: 'button', onclick: () => {
        picked.clear();
        document.querySelectorAll('.tchip[data-mode]').forEach(c => { c.removeAttribute('data-mode'); c.querySelector('.tchip-mode').textContent = ''; });
        render(); refreshCounts(bar);
      } }, 'Сбросить'),
      h('button', { class: 'btn primary', type: 'button', onclick: () => { location.hash = searchFromPicked(carry); } }, 'Найти →'));
  };
  render();
  bar._render = render;
  return bar;
}

/** Catalog chip: tap cycles  —  → ~ желательно → ! обязательно → − исключить → — */
function tagChip(t, label, bar) {
  const icon = h('span', { class: 'tchip-mode' });
  const el = h('button', {
    type: 'button', class: 'tchip', title: `${t.name} · ${t.vns} новелл`,
    onclick: () => {
      const m = NEXT_MODE[picked.get(t.id)];
      if (m) picked.set(t.id, m); else picked.delete(t.id);
      paint();
      bar._render();
      refreshCounts(bar);
    },
  }, icon, h('span', { class: 'tchip-label' }, label), h('span', { class: 'tchip-count' }, lastCounts ? (lastCounts[t.id] || 0) : t.vns));
  el.dataset.tag = t.id;
  el.title = `${t.name} · удерживайте, чтобы узнать, что это`;
  const paint = () => {
    const m = picked.get(t.id);
    if (m) el.dataset.mode = m; else el.removeAttribute('data-mode');
    icon.textContent = m ? MODE_ICON[m] : '';
    el.setAttribute('aria-label', `${label}${m ? `: ${MODE_NAME[m]}` : ''}`);
  };
  paint();
  return el;
}

export async function viewTags(root, sub, params) {
  root.append(spinner());
  let tags;
  try { tags = await api.tags(); } catch (e) { mount(root, errorBox(e)); return; }
  const tab = sub[0] === 'all' ? 'all' : (sub[0] === 'adult' && adultOn() ? 'adult' : 'catalog');
  // the catalog is the tag picker of the search: it starts from the search's tags and keeps its
  // other settings (title, platform, filters), and "Find" goes back with everything together
  const carry = new URLSearchParams(params);
  if (['must', 'want', 'exclude'].some(k => params.has(k))) {
    picked.clear();
    for (const mode of ['must', 'want', 'exclude']) {
      for (const ref of (params.get(mode) || '').split('|').filter(Boolean)) {
        const t = tags.byId.get(Number(ref)) || tags.list.find(x => x.name.toLowerCase() === ref.toLowerCase());
        if (t) picked.set(t.id, mode);
      }
    }
  }
  for (const k of ['must', 'want', 'exclude', 'f']) carry.delete(k);
  const keep = carry.toString() ? `?${carry}` : '';
  catalogCarry = carry;
  const bar = pickBar(carry);
  const filter = h('input', { type: 'search', placeholder: 'Найти тег… (по-русски или по-английски)', value: params.get('f') || '' });
  const body = h('div', { class: 'tag-browser' });
  const notAdult = t => t.cat !== 'ero';

  const groupBlock = (title, items) => h('section', { class: 'tcat' },
    h('h3', {}, title),
    h('div', { class: 'tchips' }, items.map(({ tag, label }) => tagChip(tag, label, bar))));

  const renderCatalog = () => {
    const groups = CURATED.map(g => groupBlock(g.title, resolveGroup(g, tags)));
    mount(body, h('p', { class: 'muted small' }, 'Нажмите на тег: один раз — желательно (~), два — обязательно (!), три — исключить (−). ' +
      'Удерживайте тег, чтобы узнать, что он значит. Число — сколько новелл с этим тегом останется вместе с уже выбранными обязательными и исключёнными.'), groups);
  };

  const lbl = t => (hasRu(t) ? tagLabel(t) : ruLabel(t.name));
  const treeRow = t => h('div', { class: 'tag-row' },
    t.searchable ? tagChip(t, lbl(t), bar) : h('span', { class: 'tag-plain' }, lbl(t)),
    h('a', { href: `#/tag/${t.id}`, class: 'tag-info', title: 'Описание тега' }, 'ⓘ'));

  const subtree = (t, depth, keep) => {
    const kids = t.children.map(id => tags.byId.get(id)).filter(x => x && keep(x));
    if (!kids.length) return treeRow(t);
    return h('details', { class: 'tag-node', open: depth < 1 },
      h('summary', {}, treeRow(t)),
      h('div', { class: 'tag-kids' }, kids.map(k => subtree(k, depth + 1, keep))));
  };

  const renderTree = () => {
    const roots = tags.roots.map(id => tags.byId.get(id));
    const current = tags.byId.get(Number(sub[1])) || roots[0];
    const subs = current.children.map(id => tags.byId.get(id)).filter(x => x && notAdult(x));
    mount(body,
      h('div', { class: 'tabs small-tabs' }, roots.map(r => h('a', { href: `#/tags/all/${r.id}`, class: r.id === current.id ? 'on' : '' }, ruLabel(r.name)))),
      h('div', { class: 'tag-columns' }, subs.map(s => h('section', { class: 'tag-group' },
        h('h3', {}, h('a', { href: `#/tag/${s.id}` }, ruLabel(s.name)), h('span', { class: 'count' }, s.vns || '')),
        h('div', {}, s.children.map(id => tags.byId.get(id)).filter(x => x && notAdult(x)).map(k => subtree(k, 1, notAdult)))))));
  };

  const renderAdult = () => {
    const rootEro = tags.list.find(t => t.name === 'Sexual Content');
    const isEro = t => t.cat === 'ero';
    mount(body,
      groupBlock(ADULT.title, resolveGroup(ADULT, tags)),
      rootEro ? h('section', { class: 'tcat' }, h('h3', {}, 'Все теги этой категории'),
        h('div', { class: 'tag-columns' }, rootEro.children.map(id => tags.byId.get(id)).filter(x => x && isEro(x)).map(s =>
          h('section', { class: 'tag-group' }, subtree(s, 0, isEro))))) : null);
  };

  const render = () => {
    const f = filter.value.trim();
    if (f) {
      const found = findTags(tags, f, 120, { adultOnly: tab === 'adult' });
      mount(body, found.length
        ? h('div', { class: 'tchips' }, found.filter(t => t.searchable).map(t => tagChip(t, tagLabel(t), bar)))
        : h('p', { class: 'muted' }, 'Ничего не найдено.'));
      paintCounts();
      return;
    }
    if (tab === 'all') renderTree();
    else if (tab === 'adult') renderAdult();
    else renderCatalog();
    paintCounts();
  };
  filter.addEventListener('input', debounce(render, 150));

  mount(root,
    h('div', { class: 'page-head' }, h('a', { class: 'back', href: searchFromPicked(carry) }, '← К поиску'), h('h1', {}, 'Каталог тегов')),
    h('div', { class: 'tabs' },
      h('a', { href: `#/tags${keep}`, class: tab === 'catalog' ? 'on' : '' }, 'Подборка'),
      h('a', { href: `#/tags/all${keep}`, class: tab === 'all' ? 'on' : '' }, 'Все теги VNDB'),
      adultOn() ? h('a', { href: `#/tags/adult${keep}`, class: tab === 'adult' ? 'on' : '' }, 'Для взрослых') : null),
    filter, body, bar);
  render();
  refreshCounts(bar);
}

export async function viewTag(root, id) {
  root.append(spinner());
  try {
    const [t, tags] = await Promise.all([api.tag(id), api.tags()]);
    const tt = tags.byId.get(t.id);
    const label = tagLabel(tt || t);
    const kids = t.children.map(c => tags.byId.get(c)).filter(x => x && (adultOn() || x.cat !== 'ero'));
    mount(root,
      h('div', { class: 'page-head' },
        h('div', { class: 'crumbs' }, t.path.slice(0, -1).map(pid => h('a', { href: `#/tag/${pid}` }, ruLabel(tags.byId.get(pid)?.name || String(pid))))),
        h('h1', {}, label === t.name ? ruLabel(t.name) : label),
        label !== t.name || settings().translate ? h('p', { class: 'muted' }, t.name) : null,
        t.aliases.length ? h('p', { class: 'muted small' }, 'Также: ', t.aliases.join(', ')) : null,
        h('div', { class: 'toolbar' },
          h('a', { class: 'btn', href: `#/search?want=${t.id}` }, '~ Желательно'),
          h('a', { class: 'btn', href: `#/search?must=${t.id}` }, '! Обязательно'),
          h('a', { class: 'btn', href: `#/search?exclude=${t.id}` }, '− Исключить'))),
      t.description || t.examples?.length ? tagDescription(t) : null,
      kids.length ? h('div', { class: 'subtags' }, h('span', { class: 'muted' }, 'Подтеги: '),
        kids.map(k => h('a', { class: 'pill', href: `#/tag/${k.id}` }, hasRu(k) ? tagLabel(k) : ruLabel(k.name), h('small', {}, ` ${k.vns}`)))) : null,
      h('h2', {}, 'Новеллы с этим тегом'),
      resultGrid(offset => api.search({ ...baseRequest(), must: [String(t.id)], sort: 'votes', offset, limit: 48 }), { hl: [t.id] }),
    );
  } catch (e) {
    mount(root, errorBox(e));
  }
}

let runAppCheck = () => {};
/** Result of the app update check right here (version, "Update" button, download progress). */
function appUpdateBox() {
  const box = h('div', { class: 'update-banner inline-update' }, h('span', {}, 'Проверяю, есть ли новая версия…'));
  runAppCheck = async () => {
    const m = await import('./app.js');
    const out = m.bannerOut(box, { closable: false });
    out('Проверяю, есть ли новая версия…');
    m.checkAppUpdate({ interactive: true, show: out });
  };
  setTimeout(runAppCheck, 0);
  return box;
}

// ---------- VN page ----------

const SOURCE_TITLES = {
  free: 'Скачать бесплатно (официально)',
  store: 'Купить в магазине',
  direct: 'Купить у разработчика / издателя',
  fan_patch: 'Фанатские переводы и патчи',
  original: 'Оригинал — купить или скачать',
  search: 'Ссылок на VNDB нет — поискать на',
};

const RTYPE = { trial: 'Демоверсия', partial: 'Частичный' };

function releaseCard(r) {
  const chip = (text, cls, title) => h('span', { class: `rchip ${cls}`, title: title || '' }, text);
  const chips = [
    r.official ? chip('Официальный', 'ok', 'Издан правообладателем') : chip('Фанатский', 'fan', 'Неофициальный фанатский релиз'),
    r.patch ? chip('Патч', 'plain', 'Устанавливается поверх игры') : null,
    r.freeware ? chip('Бесплатно', 'free') : null,
    RTYPE[r.rtype] ? chip(RTYPE[r.rtype], 'plain') : null,
    r.minage > 0 ? chip(`${r.minage}+`, 'plain', 'Возрастной рейтинг') : null,
  ];
  const langs = h('span', { class: 'langs' }, r.langs.map(l => h('span', {
    class: `lang k${l.mtl ? 2 : (r.official ? 0 : 1)}`,
    title: `${LANG_NAMES[l.lang] || l.lang}${l.mtl ? ' — машинный перевод' : ''}`,
  }, l.lang)));
  const sub = [r.producers.join(', '), r.platforms.map(p => PLATFORMS[p] || p).join(', ')].filter(Boolean).join(' · ');
  return h('div', { class: 'rel' },
    h('div', { class: 'rel-head' },
      h('span', { class: 'rel-date' }, r.released || 'дата неизвестна'),
      h('span', { class: 'rel-title' }, r.title)),
    sub ? h('div', { class: 'rel-sub' }, sub) : null,
    h('div', { class: 'rel-chips' }, langs, chips),
    r.links.length ? h('div', { class: 'rel-links' }, r.links.slice(0, 4).map(l =>
      h('a', { href: l.url, target: '_blank', rel: 'noopener noreferrer' }, `${l.label} ↗`))) : null);
}

function releasesBlock(d) {
  const any = settings().langMode === 'any';
  const readable = r => any || r.langs.some(l => !CJK.includes(l.lang));
  const main = d.releases.filter(readable);
  const rest = d.releases.filter(r => !readable(r));
  const section = (title, list) => list.length ? [h('h3', { class: 'rel-group' }, `${title} (${list.length})`), h('div', { class: 'rel-list' }, list.map(releaseCard))] : null;
  const official = main.filter(r => r.official);
  const fan = main.filter(r => !r.official);
  const restBox = h('div', { class: 'rel-list', hidden: true }, rest.map(releaseCard));
  const moreBtn = rest.length ? h('button', { class: 'btn', type: 'button', onclick: () => { restBox.hidden = false; moreBtn.remove(); } },
    `Показать ещё ${rest.length} (японский, китайский, корейский)`) : null;
  return h('details', { class: 'panel releases-panel' },
    h('summary', {}, `Релизы (${d.releases.length})`),
    langLegend(),
    main.length ? [section('Официальные', official), section('Фанатские переводы и патчи', fan)].filter(Boolean)
      : h('p', { class: 'muted' }, 'Релизов на читаемых языках нет.'),
    restBox, moreBtn);
}

export async function viewVn(root, id, params = new URLSearchParams()) {
  root.append(spinner());
  let d;
  let tags = null;
  try {
    [d, tags] = await Promise.all([api.vn(id), api.tags().catch(() => null)]);
  } catch (e) { mount(root, errorBox(e)); return; }
  const s = settings();
  document.title = `${d.title} — VN Shelf`;

  // personal status
  const markBox = h('label', { class: 'mark-pick' });
  const renderMarks = () => {
    const cur = d.mark && MARKS[d.mark] ? d.mark : '';
    const sel = h('select', { 'aria-label': 'Моя полка', onchange: async e => {
      const k = e.target.value;
      try {
        if (!k) { await api.unmark(d.id); d.mark = null; toast('Убрано из моих полок'); }
        else { await api.mark(d.id, k, d.note); d.mark = k; toast(`На полке «${MARKS[k].label}»`); }
      } catch (err) { toast(err.message); }
      renderMarks();
    } },
    h('option', { value: '', selected: !cur }, cur ? '✕ Убрать из полок' : '＋ В мои полки'),
    Object.entries(MARKS).map(([k, m]) => h('option', { value: k, selected: cur === k }, `${m.icon} ${m.label}`)));
    markBox.className = `mark-pick ${cur ? 'on' : ''}`;
    markBox.replaceChildren(h('span', { class: 'mark-face' }, cur ? `${MARKS[cur].icon} ${MARKS[cur].label}` : '＋ В мои полки'), h('span', { class: 'mark-caret' }, '▾'), sel);
  };
  renderMarks();

  // where to get it
  const groups = {};
  for (const src of d.sources) (groups[src.kind] ||= []).push(src);
  const priceOut = h('span', { class: 'muted small' });
  const hasSteam = d.sources.some(x => x.url.includes('store.steampowered.com/app/'));
  const where = h('section', { class: 'panel where' }, h('h2', {}, 'Где взять'),
    Object.entries(groups).map(([kind, list]) => h('div', { class: `where-group ${kind}` },
      h('h3', {}, SOURCE_TITLES[kind] || kind),
      kind === 'original' && groups.fan_patch ? h('p', { class: 'muted small' }, 'Фанатскому переводу обычно нужна купленная оригинальная игра.') : null,
      collapsible(list.map(src => h('div', { class: 'source' },
        h('a', { href: src.url, target: '_blank', rel: 'noopener noreferrer' }, src.site),
        src.langs.length ? h('span', { class: 'langs' }, src.langs.map(l => h('span', { class: 'lang' }, l))) : null,
        src.mtl ? h('span', { class: 'mini' }, 'машинный') : null,
        h('span', { class: 'price', 'data-url': src.url }),
        src.release ? h('div', { class: 'muted small' }, src.release, src.platforms.length ? ` · ${src.platforms.map(p => PLATFORMS[p] || p).join(', ')}` : '') : null)), 4))),
    communityBlock(d),
    hasSteam ? h('div', { class: 'toolbar' }, h('button', { class: 'btn', onclick: async e => {
      e.target.disabled = true; priceOut.textContent = 'Узнаю цены…';
      try {
        const cc = (navigator.language || 'en-US').split('-')[1]?.toLowerCase() || 'us';
        const r = await api.prices(d.id, cc);
        for (const p of r.prices) where.querySelectorAll(`.price[data-url="${CSS.escape(p.url)}"]`).forEach(el => { el.textContent = p.price; });
        priceOut.textContent = `Цены Steam (${r.cc.toUpperCase()})`;
      } catch (err) { priceOut.textContent = err.message; }
    } }, 'Цены в Steam'), priceOut) : null,
    h('p', { class: 'muted small' }, 'Ссылки взяты из релизов VNDB на читаемых языках. Сравнивайте: магазин (Steam и др.) или сайт разработчика.'));

  // which of the searched tags this VN has (directly or through a sub-tag)
  const hl = (params.get('hl') || '').split(',').map(Number).filter(Boolean);
  const vnTags = d.tags.flatMap(c => c.groups.flatMap(g => g.tags));
  const hitBy = new Map(); // VN tag id -> searched tag id
  const matchedVia = new Map(); // searched tag id -> [VN tags]
  if (hl.length && tags) {
    for (const t of vnTags) {
      const up = lineage(tags, t.id);
      const h1 = hl.find(x => up.has(x));
      if (h1) {
        hitBy.set(t.id, h1);
        if (!matchedVia.has(h1)) matchedVia.set(h1, []);
        matchedVia.get(h1).push(t);
      }
    }
  }
  const nameOf = tid => tagLabel(tags?.byId.get(tid) || { name: `g${tid}` });
  const hlSummary = hl.length ? h('div', { class: 'hl-summary' },
    h('span', { class: 'muted small' }, 'Теги из поиска: '),
    hl.map(tid => {
      const via = matchedVia.get(tid);
      const viaText = via ? via.filter(t => t.id !== tid).map(t => tagLabel(tags.byId.get(t.id) || t)).join(', ') : '';
      return via
        ? h('span', { class: 'hl yes', title: viaText ? `через: ${viaText}` : '' }, '✓ ', nameOf(tid), viaText ? h('small', {}, ` (${viaText})`) : null)
        : h('span', { class: 'hl no', title: 'у этой новеллы такого тега нет' }, '✗ ', nameOf(tid));
    })) : null;

  // ---- translation (description + tag names), toggled per user, cached on the server ----
  let tr = !!s.translate;
  const tagTr = new Map(); // tag id -> machine translation (tags without a curated Russian label)
  const descParts = d.description.split(/\[\/?spoiler\]/);
  let descTr = null;
  const trNote = h('span', { class: 'muted small tr-note' });
  const trBtn = h('button', { class: 'btn small-btn', type: 'button', onclick: () => {
    tr = !tr; save({ translate: tr }); paintTrBtn(); renderTags(); renderDesc(); if (tr) loadTranslations();
  } });
  const paintTrBtn = () => { trBtn.textContent = tr ? '🌐 Перевод · показать оригинал' : '🌐 Оригинал · перевести'; };
  paintTrBtn();
  const tagName = t => {
    const tt = tags?.byId.get(t.id);
    if (!tr) return t.name;
    if (tt && tagLabel(tt) !== tt.name) return tagLabel(tt);
    return tagTr.get(t.id) || t.name;
  };
  async function loadTranslations() {
    // description first (one quick request), tag names after
    if (!descTr && d.description) {
      trNote.textContent = 'перевожу…';
      try {
        descTr = (await api.translate(descParts)).translations;
        if (tr) renderDesc();
        trNote.textContent = '';
      } catch (e) {
        trNote.textContent = 'перевод недоступен (нет интернета?) — показан оригинал';
        return;
      }
    }
    const need = vnTags.filter(t => { const tt = tags?.byId.get(t.id); return !(tt && tagLabel(tt) !== tt.name) && !tagTr.has(t.id); });
    if (!need.length) return;
    try {
      const r = await api.translate(need.map(t => t.name));
      need.forEach((t, j) => tagTr.set(t.id, r.translations[j]));
      if (tr) renderTags();
    } catch { /* tags stay in English */ }
  }

  // ---- tags: same brightness for all; with "all spoilers" minor/major ones are a bit faded;
  //      tags that appear after switching the spoiler level flash briefly ----
  let spoil = Number(s.spoiler) || 0;
  const showAdult = adultOn();
  const tagBox = h('div', {});
  let shownBefore = null;
  const renderTags = (fresh = false) => {
    const nowShown = new Set();
    mount(tagBox, d.tags.map(cat => {
      const groupsEl = cat.groups.map(g => {
        const visible = g.tags.filter(t => (t.spoiler <= spoil || hitBy.has(t.id) && t.spoiler <= Math.max(spoil, 1)) && (showAdult || !t.ero));
        if (!visible.length) return null;
        visible.sort((a, b) => (hitBy.has(b.id) - hitBy.has(a.id)) || (b.rating - a.rating));
        return h('div', { class: 'tag-line' }, h('span', { class: 'tag-sub' }, tr ? ruLabel(g.name) : g.name),
          visible.map(t => {
            nowShown.add(t.id);
            const label = tagName(t);
            const cls = ['tag'];
            if (t.spoiler) cls.push('spoil', spoil === 2 ? `sp${t.spoiler}` : '');
            if (hitBy.has(t.id)) cls.push('hit');
            if (fresh && shownBefore && !shownBefore.has(t.id)) cls.push('fresh');
            return h('a', {
              href: `#/tag/${t.id}`, class: cls.join(' '), 'data-tag': t.id,
              title: `${label !== t.name ? `${t.name} · ` : ''}сила ${t.rating.toFixed(1)} из 3${t.spoiler === 1 ? ' · мелкий спойлер' : t.spoiler === 2 ? ' · серьёзный спойлер' : ''}${hitBy.has(t.id) ? ' · есть в вашем поиске' : ''} · удерживайте для описания`,
            }, hitBy.has(t.id) ? '✓ ' : '', label);
          }));
      }).filter(Boolean);
      if (!groupsEl.length) return null;
      return h('div', { class: 'tag-cat' }, h('h3', {}, tr ? ruLabel(cat.name) : catRu(cat.name)), groupsEl);
    }));
    shownBefore = nowShown;
  };
  renderTags();
  const spoilerLegend = h('div', { class: 'spoil-legend muted small' });
  const paintLegend = () => mount(spoilerLegend, spoil === 2 ? [
    h('span', { class: 'tag' }, 'обычный'), h('span', { class: 'tag spoil sp1' }, 'мелкий спойлер'), h('span', { class: 'tag spoil sp2' }, 'серьёзный спойлер'),
  ] : null);
  paintLegend();
  const tagControls = h('div', { class: 'toolbar' },
    h('label', { class: 'field inline' }, 'Спойлеры ', h('select', { onchange: e => { spoil = Number(e.target.value); paintLegend(); renderTags(true); } },
      [['0', 'скрыть'], ['1', 'мелкие'], ['2', 'все']].map(([v, l]) => h('option', { value: v, selected: Number(v) === spoil }, l)))));

  // description with spoiler blocks (original or translated)
  const desc = h('div', { class: 'desc' });
  const renderDesc = () => {
    if (!(tr && descTr) && d.description_parts) { mount(desc, richText(d.description_parts)); return; }
    const parts = tr && descTr ? descTr : descParts;
    mount(desc, parts.map((part, i) => (i % 2
      ? h('span', { class: 'spoiler', title: 'Спойлер — нажмите', onclick: e => e.currentTarget.classList.add('shown') }, part)
      : part)));
  };
  renderDesc();
  if (tr) loadTranslations();

  // screenshots: sharp thumbnails, full picture in the viewer
  const shotList = d.screenshots.slice(0, 40);
  const gallery = shotList.map(sc => ({ src: img(sc.full), alt: d.title }));
  const shots = shotList.length ? h('section', {}, h('h2', {}, `Скриншоты (${d.screenshots.length})`),
    h('div', { class: 'shots' }, shotList.map((sc, i) => {
      const pic = cover(sc.thumb, sc.sexual, `Скриншот ${i + 1}`, 'shot', { full: sc.full, thumbW: 200, fullW: 1280, sizes: '(max-width: 760px) 46vw, 280px' });
      return h('button', { class: 'shot-btn', type: 'button', 'aria-label': `Скриншот ${i + 1}`,
        onclick: () => { if (!pic.classList.contains('blurred')) openLightbox(gallery, i); } }, pic);
    }))) : null;

  const rels = d.relations.length ? h('section', {}, h('h2', {}, 'Связанные'),
    h('div', { class: 'pills' }, d.relations.map(r => h('a', { class: 'pill', href: `#/vn/${r.id}` }, r.title, h('small', {}, ` · ${relName(r.relation)}`))))) : null;

  const meta = [
    d.released && ['Вышла', d.released],
    d.rating && ['Рейтинг', `★ ${d.rating.toFixed(2)} (${d.votes.toLocaleString('ru')} голосов)`],
    d.length_minutes && ['Длина', `${fmtLength(d.length_minutes)} (${d.length_votes} оценок)`],
    d.developers.length && ['Разработчик', d.developers.join(', ')],
    d.devstatus === 1 && ['Статус', 'в разработке'],
    d.devstatus === 2 && ['Статус', 'отменена'],
  ].filter(Boolean);

  const coverEl = cover(d.image_full || d.image, d.img_sexual, d.title, 'cover big');
  const coverBtn = h('button', { class: 'cover-btn', type: 'button', 'aria-label': 'Открыть обложку',
    onclick: () => { if (!coverEl.classList.contains('blurred') && d.image_full) openLightbox([{ src: img(d.image_full), alt: d.title }]); } }, coverEl);

  mount(root,
    // first screen: small cover + title + key facts, one status button, then "where to get"
    h('div', { class: 'vn-hero' },
      h('div', { class: 'vn-cover' }, coverBtn),
      h('div', { class: 'vn-info' },
        h('h1', {}, d.title),
        d.original && d.original !== d.title ? h('p', { class: 'muted vn-original' }, d.original) : null,
        h('dl', { class: 'meta' }, meta.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])))),
    h('div', { class: 'vn-actions' }, markBox,
      h('a', { class: 'btn', href: d.vndb_url, target: '_blank', rel: 'noopener' }, 'VNDB ↗')),
    hlSummary,
    where,
    h('section', { class: 'panel vn-facts' },
      d.platforms.length ? h('p', {}, h('span', { class: 'muted' }, 'Платформы: '), d.platforms.map(p => PLATFORMS[p] || p).join(', ')) : null,
      h('div', { class: 'lang-line' }, h('span', { class: 'muted' }, 'Языки: '), langBadges(d.langs, 30)),
      h('details', { class: 'legend-box' }, h('summary', { class: 'muted small' }, 'Что значат значки языков'), langLegend(d.langs)),
      d.links.some(l => ['wikidata', 'wp'].indexOf(l.site) < 0) ? h('div', { class: 'toolbar' },
        d.links.filter(l => ['wikidata', 'wp'].indexOf(l.site) < 0).slice(0, 4).map(l => h('a', { class: 'btn', href: l.url, target: '_blank', rel: 'noopener noreferrer' }, l.label, ' ↗'))) : null),
    h('div', { class: 'tr-line' }, trBtn, trNote),
    d.description ? h('section', {}, h('h2', {}, 'Описание'), desc) : null,
    h('section', {}, h('div', { class: 'section-head' }, h('h2', {}, 'Теги'), tagControls), spoilerLegend, tagBox),
    shots, rels, releasesBlock(d),
  );
}

function communityBlock(d) {
  const list = d.community || [];
  if (!list.length) return null;
  const direct = list.filter(l => l.direct);
  const search = list.filter(l => !l.direct);
  return h('div', { class: 'where-group community' },
    h('h3', {}, 'Каталоги переводов и сообщества'),
    direct.map(l => h('div', { class: 'source' },
      h('a', { href: l.url, target: '_blank', rel: 'noopener noreferrer' }, `${l.name} — страница новеллы`),
      h('span', { class: 'langs' }, h('span', { class: 'lang' }, l.lang)))),
    search.length ? h('div', { class: 'source' }, h('span', { class: 'muted small' }, 'Поискать: '),
      search.map((l, i) => [i ? ' · ' : '', h('a', { href: l.url, target: '_blank', rel: 'noopener noreferrer' }, l.name)])) : null);
}

function collapsible(items, max) {
  if (items.length <= max) return items;
  const rest = h('div', { hidden: true }, items.slice(max));
  const btn = h('button', { class: 'linkish', onclick: () => { rest.hidden = false; btn.remove(); } }, `ещё ${items.length - max}…`);
  return [...items.slice(0, max), rest, btn];
}

function relName(r) {
  return { seq: 'продолжение', preq: 'предыстория', set: 'тот же мир', alt: 'альт. версия', char: 'общие персонажи',
    side: 'побочная история', par: 'основная история', ser: 'та же серия', fan: 'фанфик', orig: 'оригинал' }[r] || r;
}

// ---------- personal shelves ----------

export async function viewMy(root, status) {
  root.append(spinner());
  try {
    const my = await api.my();
    const current = status && MARKS[status] ? status : null;
    const counts = {};
    for (const m of my.marks) counts[m.status] = (counts[m.status] || 0) + 1;
    const ids = my.marks.filter(m => !current || m.status === current).map(m => Number(m.id.slice(1)));
    mount(root,
      h('div', { class: 'page-head' }, h('h1', {}, 'Мои полки')),
      h('div', { class: 'tabs wrap' },
        h('a', { href: '#/my', class: current ? '' : 'on' }, `Все (${my.marks.length})`),
        Object.entries(MARKS).map(([k, m]) => h('a', { href: `#/my/${k}`, class: current === k ? 'on' : '' }, `${m.icon} ${m.label} (${counts[k] || 0})`))),
      ids.length
        ? resultGrid(offset => api.search({ ...baseRequest(), lang_mode: 'any', ero: 'show', ids, sort: 'title', offset, limit: 48 }))
        : h('p', { class: 'muted center' }, 'Здесь пусто. Откройте любую новеллу и нажмите «Хочу», «Читаю» и т.д.'),
    );
  } catch (e) {
    mount(root, errorBox(e));
  }
}

// ---------- settings ----------

export async function viewSettings(root) {
  const s = settings();
  const field = (label, control, hint) => h('label', { class: 'field' }, h('span', {}, label), control, hint ? h('small', { class: 'muted' }, hint) : null);
  const select = (key, options) => h('select', { onchange: e => { save({ [key]: e.target.value }); toast('Сохранено'); } },
    options.map(([v, l]) => h('option', { value: v, selected: String(s[key]) === String(v) }, l)));
  const check = (key, label, hint) => h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: !!s[key], onchange: e => { save({ [key]: e.target.checked }); toast('Сохранено'); } }),
    h('span', {}, label, hint ? h('small', { class: 'muted block' }, hint) : null));
  const text = (key, ph) => {
    const i = h('input', { type: 'text', value: s[key], placeholder: ph });
    i.addEventListener('change', () => { save({ [key]: i.value.trim() }); toast('Сохранено'); });
    return i;
  };

  const srcBox = h('div', {}, spinner());
  const loadSources = async () => {
    try {
      const r = await api.sources();
      mount(srcBox,
        h('p', { class: 'muted small' }, 'Ссылки на эти сайты появляются в блоке «Где взять». Для сайтов с индексом даётся прямая ссылка на страницу новеллы, для остальных — поиск. Список можно расширить файлом sources.json в папке данных.'),
        h('ul', { class: 'src-list' }, r.sources.map(x => h('li', {},
          h('b', {}, x.name), ` (${x.lang}) `,
          h('span', { class: 'muted small' }, x.kind === 'store_search' ? 'поиск по магазину, если на VNDB нет ссылок' : (x.description || '')),
          x.indexed ? h('div', { class: 'muted small' }, x.entries ? `индекс: ${x.entries} страниц, обновлён ${x.fetched_at}` : 'индекс ещё не загружен') : null))),
        h('button', { class: 'btn', onclick: async () => { await api.refreshSources(); toast('Обновляю индексы'); setTimeout(loadStatus, 800); } }, 'Обновить индексы'));
    } catch (e) { mount(srcBox, errorBox(e, loadSources)); }
  };
  loadSources();
  const dbBox = h('div', {}, spinner());
  const dbNote = h('div', { class: 'update-banner inline-update', hidden: true });
  const loadStatus = async () => {
    try {
      const st = await api.status();
      const full = async e => {
        e.currentTarget.disabled = true; await api.startImport(); toast('Загрузка базы запущена'); setTimeout(loadStatus, 1000);
      };
      // one button: new releases through the API, or a full download when the gap is big (see the policy below)
      const actions = st.ready
        ? h('div', { class: 'toolbar' },
          h('button', { class: 'btn primary', disabled: st.import.running, onclick: async () => {
            const m = await import('./app.js');
            const out = m.bannerOut(dbNote, { closable: false });
            out('Проверяю VNDB…');
            m.checkUpdates({ force: true, interactive: true, show: out, onStart: () => setTimeout(loadStatus, 800) });
          } }, 'Проверить обновления базы'),
          h('button', { class: 'linkish small', disabled: st.import.running, onclick: full }, 'Скачать базу заново целиком'))
        : h('button', { class: 'btn primary', disabled: st.import.running, onclick: full }, 'Скачать базу VNDB');
      mount(dbBox,
        h('p', {}, st.ready ? `В базе ${st.vns.toLocaleString('ru')} новелл и ${st.tags} тегов. Дамп VNDB от ${st.dump_timestamp}, импорт ${st.imported_at} (UTC).` : 'База не загружена.'),
        actions, (st.import.running || st.import.error) ? jobPanel(st.import) : (st.import.kind ? h('div', { class: 'muted small' }, `Последняя задача: ${({ import: 'полная загрузка', sync: 'новые релизы', sources: 'индексы каталогов' })[st.import.kind] || st.import.kind} — ${st.import.finished ? 'готово' : '…'}`) : null));
      if (st.import.running) setTimeout(loadStatus, 1000);
      else if (st.ready && (st.import.log || []).includes('Ready')) api.resetTags();
    } catch (e) { mount(dbBox, errorBox(e, loadStatus)); }
  };
  loadStatus();

  const reset = h('button', { class: 'btn', onclick: () => { save({ ...DEFAULTS }); toast('Настройки сброшены'); location.reload(); } }, 'Сбросить настройки');
  root.append(
    h('div', { class: 'page-head' }, h('h1', {}, 'Настройки')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Язык'),
      field('Предпочитаемые языки', text('langs', 'en, ru'), 'Коды через запятую по приоритету: en, ru, uk, es, de… Используются для названий и сортировки ссылок.'),
      field('Какие новеллы показывать', select('langMode', [['readable', 'Есть на любом языке, кроме японского/китайского/корейского'], ['list', 'Только на языках из списка выше'], ['any', 'Все, даже только на японском']])),
      check('allowMtl', 'Учитывать машинные переводы'),
      check('translate', 'Переводить описания и теги на русский', 'Машинный перевод (нужен интернет при первом показе, потом из кэша). На странице новеллы можно переключать «оригинал / перевод».')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Где играть'),
      field('Показывать новеллы для', select('platform', [['any', 'Любой платформы'], ['pc', 'ПК (Windows, Linux, macOS)'], ['phone', 'Телефона (Android, iOS)']]),
        'Действует на полки и поиск; в поиске можно переключить на месте.')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Контент'),
      field('Спойлеры в тегах', select('spoiler', [['0', 'Скрывать'], ['1', 'Показывать мелкие'], ['2', 'Показывать все']])),
      check('hideCancelled', 'Скрывать отменённые проекты'),
      check('adultTags', 'Теги для взрослых', 'Отдельный список в каталоге тегов, в поиске и на страницах новелл.'),
      check('hideAdult', 'Скрывать новеллы с откровенными сценами'),
      field('Размывать обложки и скриншоты', select('blur', [['explicit', 'Только откровенные'], ['suggestive', 'Откровенные и пикантные'], ['off', 'Не размывать']]))),
    h('section', { class: 'panel form' }, h('h2', {}, 'Вид'),
      field('Тема', select('theme', [['auto', 'Как в системе'], ['dark', 'Тёмная'], ['light', 'Светлая']]))),
    h('section', { class: 'panel form' }, h('h2', {}, 'База данных и обновления'), dbBox, dbNote,
      field('Обновления', select('updatePolicy', [
        ['smart', 'Новые релизы — сами, полную загрузку — спрашивать'],
        ['auto', 'Всё автоматически (и полную загрузку тоже)'],
        ['manual', 'Только вручную']])),
      ),
    h('section', { class: 'panel form' }, h('h2', {}, 'Каталоги переводов и сообщества'), srcBox),
    IN_APP
      ? h('section', { class: 'panel form' }, h('h2', {}, 'Приложение'),
        h('p', {}, `VN Shelf для Android, версия ${APP_VERSION}. Новые версии публикуются в GitHub Releases.`),
        appUpdateBox(),
        h('div', { class: 'toolbar' },
          h('button', { class: 'btn', onclick: () => runAppCheck() }, 'Проверить снова'),
          reset))
      : h('section', { class: 'panel form' }, h('h2', {}, 'Сервер'),
        field('Адрес сервера', text('apiBase', 'пусто = этот же сервер'), 'Нужно для мобильного приложения: например http://192.168.1.10:8080'),
        h('p', { class: 'muted small' }, 'Чтобы открыть с телефона: запустите на ПК «vndb-web serve --addr 0.0.0.0:8080» и откройте http://IP-компьютера:8080 в браузере телефона. ' +
          'Там же можно «Установить приложение» / «На экран Домой».'),
        reset),
  );
}
