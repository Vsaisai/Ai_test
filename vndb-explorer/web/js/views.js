// All screens of the app. Each view renders into `root` and may return a cleanup function.
import { api } from './api.js';
import { settings, save, baseRequest, langList, DEFAULTS, IN_APP, APP_VERSION } from './store.js';
import {
  h, card, cover, spinner, errorBox, toast, debounce, langBadges, fmtLength,
  LANG_NAMES, PLATFORMS, KIND, MARKS, ROOT_RU, shouldBlur, jobPanel,
} from './ui.js';
import { img } from './api.js';

const SORTS = {
  relevance: 'По соответствию', votes: 'Популярные', rating: 'Рейтинг', newest: 'Новые', oldest: 'Старые',
  title: 'По алфавиту', shortest: 'Короткие', longest: 'Длинные',
};

// ---------- tags helpers ----------

export function tagPath(tags, id) {
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

export function findTags(tags, q, limit = 12) {
  const s = q.trim().toLowerCase();
  if (!s) return [];
  const adult = settings().adult;
  const scored = [];
  for (const t of tags.list) {
    if (!adult && t.cat === 'ero') continue;
    const n = t.name.toLowerCase();
    let score = -1;
    if (n === s) score = 100;
    else if (n.startsWith(s)) score = 60;
    else if (n.includes(s)) score = 30;
    else if (t.aliases.some(a => a.toLowerCase().includes(s))) score = 20;
    if (score < 0) continue;
    scored.push([score + Math.min(Math.log10(t.vns + 1) * 5, 19), t]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  return scored.slice(0, limit).map(x => x[1]);
}

/** Autocomplete input for tags. `onPick(tag, mode)`; mode is 'must' | 'want' | 'exclude'. */
function tagPicker(tags, onPick, defaultMode = 'want') {
  let mode = defaultMode;
  const list = h('div', { class: 'suggest', hidden: true });
  const input = h('input', { type: 'search', placeholder: 'Добавить тег… (например, time travel)', autocomplete: 'off' });
  const modes = h('div', { class: 'seg' }, ['must', 'want', 'exclude'].map(m =>
    h('button', {
      type: 'button', class: m === mode ? 'on' : '', 'data-m': m,
      onclick: e => { mode = m; modes.querySelectorAll('button').forEach(b => b.classList.toggle('on', b.dataset.m === m)); input.focus(); },
    }, { must: 'Обязательно', want: 'Желательно', exclude: 'Исключить' }[m])));
  let active = -1;
  const render = () => {
    const found = findTags(tags, input.value);
    list.replaceChildren(...found.map((t, i) => h('button', {
      type: 'button', class: `sug ${i === active ? 'active' : ''}`,
      onmousedown: e => { e.preventDefault(); pick(t); },
    }, h('span', { class: 'sug-name' }, t.name, t.cat === 'ero' ? h('span', { class: 'mini ero' }, '18+') : null),
      h('span', { class: 'sug-path' }, tagPath(tags, t.id).slice(0, -1).map(x => x.name).join(' › ')),
      h('span', { class: 'sug-count' }, t.vns))));
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

function resultGrid(loader, { wantCount = 0, empty = 'Ничего не найдено.' } = {}) {
  const grid = h('div', { class: 'grid' });
  const info = h('div', { class: 'result-info' });
  const more = h('button', { class: 'btn wide', hidden: true }, 'Показать ещё');
  const wrap = h('div', {}, info, grid, more);
  let offset = 0;
  let total = 0;
  const load = async () => {
    more.disabled = true;
    const sp = spinner();
    wrap.append(sp);
    try {
      const r = await loader(offset);
      total = r.total;
      offset += r.items.length;
      grid.append(...r.items.map(it => card(it, { wantCount })));
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
  const s = settings();
  root.append(spinner('Раскладываем по полкам…'));
  let st;
  try {
    st = await api.status();
  } catch (e) {
    root.replaceChildren(errorBox(e, () => location.reload()));
    return;
  }
  if (!st.ready) {
    root.replaceChildren(emptyDb(st));
    return;
  }
  try {
    const data = await api.shelves(baseRequest(), s.adult, 16);
    root.replaceChildren(
      h('div', { class: 'home-head' },
        h('p', { class: 'muted' }, `${st.vns.toLocaleString('ru')} визуальных новелл · база от ${(st.dump_timestamp || '').slice(0, 10)}`),
        h('div', { class: 'row-actions' },
          h('a', { class: 'btn', href: '#/search' }, '🔎 Поиск по тегам'),
          h('a', { class: 'btn', href: '#/tags' }, '🏷️ Все теги'),
          h('button', { class: 'btn', onclick: async () => {
            const r = await api.random({ ...baseRequest(), min_votes: 20 });
            if (r.id) location.hash = `#/vn/${r.id}`;
          } }, '🎲 Случайная'))),
      ...data.sections.filter(sec => sec.shelves.length).map(sec => h('section', { class: 'shelf-section' },
        h('h2', {}, sec.title),
        ...sec.shelves.map(sh => h('div', { class: 'shelf' },
          h('div', { class: 'shelf-head' },
            h('a', { href: `#/shelf/${sh.id}`, class: 'shelf-title' }, sh.title),
            h('a', { href: `#/shelf/${sh.id}`, class: 'shelf-all' }, `все ${sh.total.toLocaleString('ru')} →`)),
          h('div', { class: 'shelf-row' }, sh.items.map(it => card(it)))))),
      ),
    );
  } catch (e) {
    root.replaceChildren(errorBox(e, () => viewHome(root)));
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
    h('p', {}, 'Приложение работает офлайн с полной копией VNDB (все новеллы, теги, релизы, ссылки, включая 18+). ' +
      'Скачайте дамп один раз — дальше всё работает без интернета (кроме обложек при первом показе).'),
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
  };
}

function writeSearch(st) {
  const p = new URLSearchParams();
  if (st.q) p.set('q', st.q);
  for (const k of ['must', 'want', 'exclude', 'plat']) if (st[k].length) p.set(k, st[k].join('|'));
  if (st.sort && st.sort !== 'relevance') p.set('sort', st.sort);
  for (const k of ['lmin', 'lmax', 'yfrom', 'yto', 'votes', 'rating', 'mm']) if (st[k] !== '' && st[k] !== undefined) p.set(k, st[k]);
  if (st.free) p.set('free', '1');
  return `#/search?${p}`;
}

export async function viewSearch(root, params) {
  root.append(spinner());
  let tags;
  try { tags = await api.tags(); } catch (e) { root.replaceChildren(errorBox(e)); return; }
  root.replaceChildren();
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
      return h('span', { class: `chip ${k}`, title: tagPath(tags, id).map(x => x.name).join(' › ') },
        h('button', {
          type: 'button', class: 'chip-mode', title: 'Сменить: обязательно → желательно → исключить',
          onclick: () => {
            const next = { must: 'want', want: 'exclude', exclude: 'must' }[k];
            st[k] = st[k].filter(x => x !== id); st[next].push(id); go();
          },
        }, { must: '!', want: '~', exclude: '−' }[k]),
        t ? t.name : `g${id}`,
        h('button', { type: 'button', class: 'chip-x', title: 'Убрать', onclick: () => { st[k] = st[k].filter(x => x !== id); go(); } }, '×'));
    })));
    if (!chips.children.length) chips.append(h('span', { class: 'muted' }, 'Теги не выбраны. «!» — обязательно, «~» — желательно, «−» — исключить. Нажмите на значок, чтобы сменить.'));
  };
  renderChips();

  const text = h('input', { type: 'search', value: st.q, placeholder: 'Название (необязательно)' });
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
      h('div', { class: 'field' }, h('span', {}, 'Мин. желательных тегов'), num('mm', '1')),
      h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: st.free, onchange: e => { st.free = e.target.checked; go(); } }), 'Только бесплатные'),
      h('div', { class: 'field full' }, h('span', {}, 'Платформы'), platSel)),
    h('p', { class: 'muted small' }, 'Язык, 18+ и спойлеры настраиваются в «Настройках».'));

  root.append(
    h('div', { class: 'page-head' }, h('h1', {}, 'Поиск')),
    h('div', { class: 'panel search-panel' },
      text,
      tagPicker(tags, (t, mode) => {
        for (const k of ['must', 'want', 'exclude']) st[k] = st[k].filter(x => x !== t.id);
        st[mode].push(t.id);
        go();
      }),
      chips,
      filters,
      h('div', { class: 'toolbar' },
        sortSelect(st.sort, v => { st.sort = v; go(); }),
        h('button', { class: 'btn', onclick: () => { location.hash = '#/search'; } }, 'Сбросить'))),
  );
  if (!st.must.length && !st.want.length && !st.exclude.length && !st.q && !st.plat.length && !st.free && !st.lmax && !st.lmin) {
    root.append(h('p', { class: 'muted center' }, 'Добавьте теги или название. Подсказка: можно выбрать теги в разделе «Теги».'));
    return;
  }
  const req = {
    ...baseRequest(),
    text: st.q || null,
    must: st.must.map(String), want: st.want.map(String), exclude: st.exclude.map(String),
    sort: st.sort,
    platforms: st.plat,
    length_min: st.lmin ? Number(st.lmin) : null, length_max: st.lmax ? Number(st.lmax) : null,
    year_from: st.yfrom ? Number(st.yfrom) : null, year_to: st.yto ? Number(st.yto) : null,
    min_votes: st.votes ? Number(st.votes) : null, min_rating: st.rating ? Number(st.rating) : null,
    freeware: st.free ? true : null,
    min_match: st.mm !== '' ? Number(st.mm) : null,
  };
  root.append(resultGrid(offset => api.search({ ...req, offset, limit: 48 }), { wantCount: st.want.length }));
}

// ---------- tags browser ----------

export async function viewTags(root, rootId, params) {
  root.append(spinner());
  let tags;
  try { tags = await api.tags(); } catch (e) { root.replaceChildren(errorBox(e)); return; }
  const adult = settings().adult;
  const roots = tags.roots.map(id => tags.byId.get(id));
  const current = tags.byId.get(Number(rootId)) || roots[0];
  const filter = h('input', { type: 'search', placeholder: 'Фильтр тегов…', value: params.get('f') || '' });
  const body = h('div', { class: 'tag-browser' });
  const visible = t => adult || t.cat !== 'ero';

  const tagRow = t => h('div', { class: 'tag-row' },
    h('a', { href: `#/tag/${t.id}`, class: `tag-link ${t.cat === 'ero' ? 'ero' : ''}` }, t.name),
    h('span', { class: 'count' }, t.vns || ''),
    t.searchable ? h('span', { class: 'tag-actions' },
      h('a', { href: `#/search?must=${t.id}`, title: 'Обязательно' }, '!'),
      h('a', { href: `#/search?want=${t.id}`, title: 'Желательно' }, '~'),
      h('a', { href: `#/search?exclude=${t.id}`, title: 'Исключить' }, '−')) : null);

  const subtree = (t, depth) => {
    const kids = t.children.map(id => tags.byId.get(id)).filter(x => x && visible(x));
    if (!kids.length) return tagRow(t);
    return h('details', { class: 'tag-node', open: depth < 1 },
      h('summary', {}, tagRow(t)),
      h('div', { class: 'tag-kids' }, kids.map(k => subtree(k, depth + 1))));
  };

  const render = () => {
    const f = filter.value.trim();
    if (f) {
      const found = findTags(tags, f, 200);
      body.replaceChildren(h('div', { class: 'tag-flat' }, found.map(t => h('div', { class: 'tag-found' },
        tagRow(t), h('div', { class: 'sug-path' }, tagPath(tags, t.id).slice(0, -1).map(x => x.name).join(' › '))))));
      return;
    }
    const subs = current.children.map(id => tags.byId.get(id)).filter(x => x && visible(x));
    body.replaceChildren(h('div', { class: 'tag-columns' }, subs.map(s => h('section', { class: 'tag-group' },
      h('h3', {}, h('a', { href: `#/tag/${s.id}` }, s.name), h('span', { class: 'count' }, s.vns || '')),
      h('div', {}, s.children.map(id => tags.byId.get(id)).filter(x => x && visible(x)).map(k => subtree(k, 1)))))));
  };
  filter.addEventListener('input', debounce(render, 150));

  root.replaceChildren(
    h('div', { class: 'page-head' }, h('h1', {}, 'Теги по категориям'),
      h('p', { class: 'muted' }, 'Все теги VNDB разложены по категориям. «!» — обязательно, «~» — желательно, «−» — исключить.')),
    h('div', { class: 'tabs' }, roots.map(r => h('a', { href: `#/tags/${r.id}`, class: r.id === current.id ? 'on' : '' }, ROOT_RU[r.name] || r.name))),
    filter, body);
  render();
}

export async function viewTag(root, id) {
  root.append(spinner());
  try {
    const [t, tags] = await Promise.all([api.tag(id), api.tags()]);
    const kids = t.children.map(c => tags.byId.get(c)).filter(x => x && (settings().adult || x.cat !== 'ero'));
    root.replaceChildren(
      h('div', { class: 'page-head' },
        h('div', { class: 'crumbs' }, t.path.slice(0, -1).map(pid => h('a', { href: `#/tag/${pid}` }, tags.byId.get(pid)?.name || pid))),
        h('h1', {}, t.name, t.cat === 'ero' ? h('span', { class: 'mini ero' }, '18+') : null),
        t.aliases.length ? h('p', { class: 'muted' }, 'Также: ', t.aliases.join(', ')) : null,
        h('div', { class: 'toolbar' },
          h('a', { class: 'btn', href: `#/search?must=${t.id}` }, '! Обязательно'),
          h('a', { class: 'btn', href: `#/search?want=${t.id}` }, '~ Желательно'),
          h('a', { class: 'btn', href: `#/search?exclude=${t.id}` }, '− Исключить'))),
      t.description ? h('p', { class: 'desc' }, t.description) : null,
      kids.length ? h('div', { class: 'subtags' }, h('span', { class: 'muted' }, 'Подтеги: '),
        kids.map(k => h('a', { class: 'pill', href: `#/tag/${k.id}` }, k.name, h('small', {}, ` ${k.vns}`)))) : null,
      h('h2', {}, 'Новеллы с этим тегом'),
      resultGrid(offset => api.search({ ...baseRequest(), must: [String(t.id)], sort: 'votes', offset, limit: 48 })),
    );
  } catch (e) {
    root.replaceChildren(errorBox(e));
  }
}

// ---------- VN page ----------

const SOURCE_TITLES = {
  free: 'Скачать бесплатно (официально)',
  store: 'Купить в магазине',
  direct: 'Купить у разработчика / издателя',
  fan_patch: 'Фанатские переводы и патчи',
  search: 'На VNDB ссылок нет — поискать на',
};

export async function viewVn(root, id) {
  root.append(spinner());
  let d;
  try { d = await api.vn(id); } catch (e) { root.replaceChildren(errorBox(e)); return; }
  const s = settings();
  document.title = `${d.title} — VN Shelf`;

  // personal status
  const markBox = h('div', { class: 'marks' });
  const renderMarks = () => markBox.replaceChildren(...Object.entries(MARKS).map(([k, m]) => h('button', {
    class: `btn mark ${d.mark === k ? 'on' : ''}`, type: 'button',
    onclick: async () => {
      try {
        if (d.mark === k) { await api.unmark(d.id); d.mark = null; toast('Убрано с полки'); }
        else { await api.mark(d.id, k, d.note); d.mark = k; toast(`На полке «${m.label}»`); }
        renderMarks();
      } catch (e) { toast(e.message); }
    },
  }, m.icon, ' ', m.label)));
  renderMarks();

  // where to get it
  const groups = {};
  for (const src of d.sources) (groups[src.kind] ||= []).push(src);
  const priceOut = h('span', { class: 'muted small' });
  const hasSteam = d.sources.some(x => x.url.includes('store.steampowered.com/app/'));
  const where = h('section', { class: 'panel where' }, h('h2', {}, 'Где взять'),
    Object.entries(groups).map(([kind, list]) => h('div', { class: `where-group ${kind}` },
      h('h3', {}, SOURCE_TITLES[kind] || kind),
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

  // tags
  let spoil = Number(s.spoiler) || 0;
  let showEro = s.adult;
  const tagBox = h('div', {});
  const renderTags = () => {
    tagBox.replaceChildren(...d.tags.map(cat => {
      const groupsEl = cat.groups.map(g => {
        const visible = g.tags.filter(t => t.spoiler <= spoil && (showEro || !t.ero));
        if (!visible.length) return null;
        return h('div', { class: 'tag-line' }, h('span', { class: 'tag-sub' }, g.name),
          visible.map(t => h('a', {
            href: `#/tag/${t.id}`, class: `tag t${Math.min(3, Math.round(t.rating))} ${t.ero ? 'ero' : ''} ${t.spoiler ? 'spoil' : ''}`,
            title: `${t.rating.toFixed(1)}${t.spoiler ? ' · спойлер' : ''}`,
          }, t.name)));
      }).filter(Boolean);
      if (!groupsEl.length) return null;
      return h('div', { class: 'tag-cat' }, h('h3', {}, ROOT_RU[cat.name] || cat.name), groupsEl);
    }).filter(Boolean));
  };
  renderTags();
  const tagControls = h('div', { class: 'toolbar' },
    h('label', { class: 'field inline' }, 'Спойлеры ', h('select', { onchange: e => { spoil = Number(e.target.value); renderTags(); } },
      [['0', 'скрыть'], ['1', 'мелкие'], ['2', 'все']].map(([v, l]) => h('option', { value: v, selected: Number(v) === spoil }, l)))),
    h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: showEro, onchange: e => { showEro = e.target.checked; renderTags(); } }), '18+ теги'));

  // description with spoiler blocks
  const desc = h('div', { class: 'desc' });
  for (const [i, part] of d.description.split(/\[\/?spoiler\]/).entries()) {
    if (i % 2) desc.append(h('span', { class: 'spoiler', title: 'Спойлер — нажмите', onclick: e => e.currentTarget.classList.add('shown') }, part));
    else desc.append(part);
  }

  const shots = d.screenshots.length ? h('section', {}, h('h2', {}, 'Скриншоты'),
    h('div', { class: 'shots' }, d.screenshots.slice(0, 24).map(sc => h('a', { href: img(sc.full), target: '_blank', rel: 'noopener' },
      cover(sc.thumb, sc.sexual, '', 'shot'))))) : null;

  const rels = d.relations.length ? h('section', {}, h('h2', {}, 'Связанные'),
    h('div', { class: 'pills' }, d.relations.map(r => h('a', { class: 'pill', href: `#/vn/${r.id}` }, r.title, h('small', {}, ` · ${relName(r.relation)}`))))) : null;

  const relRow = r => h('tr', {},
    h('td', {}, r.released || '—'),
    h('td', {}, r.title, r.producers.length ? h('div', { class: 'muted small' }, r.producers.join(', ')) : null),
    h('td', {}, r.langs.map(l => `${l.lang}${l.mtl ? '(MTL)' : ''}`).join(' ')),
    h('td', {}, r.platforms.join(' ')),
    h('td', {}, [r.official ? 'офиц.' : 'фан', r.patch ? 'патч' : '', r.freeware ? 'бесплатно' : '',
      r.rtype !== 'complete' ? r.rtype : '', r.minage >= 18 ? '18+' : ''].filter(Boolean).join(', ')),
    h('td', {}, r.links.slice(0, 3).map(l => h('a', { href: l.url, target: '_blank', rel: 'noopener noreferrer', class: 'small' }, l.label, ' '))));
  const relHead = h('thead', {}, h('tr', {}, ['Дата', 'Название', 'Языки', 'Платформы', 'Тип', ''].map(x => h('th', {}, x))));
  const releases = h('details', { class: 'panel' }, h('summary', {}, `Все релизы (${d.releases.length})`),
    h('div', { class: 'table-wrap' }, h('table', { class: 'releases' }, relHead, h('tbody', {}, d.releases.map(relRow)))));

  const meta = [
    d.released && ['Вышла', d.released],
    d.rating && ['Рейтинг', `★ ${d.rating.toFixed(2)} (${d.votes.toLocaleString('ru')} голосов)`],
    d.length_minutes && ['Длина', `${fmtLength(d.length_minutes)} (${d.length_votes} оценок)`],
    d.developers.length && ['Разработчик', d.developers.join(', ')],
    d.platforms.length && ['Платформы', d.platforms.map(p => PLATFORMS[p] || p).join(', ')],
    d.devstatus === 1 && ['Статус', 'в разработке'],
    d.devstatus === 2 && ['Статус', 'отменена'],
  ].filter(Boolean);

  root.replaceChildren(
    h('div', { class: 'vn-hero' },
      h('div', { class: 'vn-cover' }, cover(d.image_full || d.image, d.img_sexual, d.title, 'cover big')),
      h('div', { class: 'vn-info' },
        h('h1', {}, d.title, d.ero ? h('span', { class: 'mini ero' }, '18+') : null),
        d.original && d.original !== d.title ? h('p', { class: 'muted' }, d.original) : null,
        h('dl', { class: 'meta' }, meta.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])),
        h('div', { class: 'lang-line' }, h('span', { class: 'muted small' }, 'Языки: '), langBadges(d.langs, 30),
          h('span', { class: 'muted small legend' }, ' сплошная — офиц., контур — фан-перевод, пунктир — машинный')),
        markBox,
        h('div', { class: 'toolbar' }, h('a', { class: 'btn', href: d.vndb_url, target: '_blank', rel: 'noopener' }, 'VNDB ↗'),
          d.links.filter(l => ['wikidata', 'wp'].indexOf(l.site) < 0).slice(0, 4).map(l => h('a', { class: 'btn', href: l.url, target: '_blank', rel: 'noopener noreferrer' }, l.label, ' ↗'))))),
    where,
    d.description ? h('section', {}, h('h2', {}, 'Описание'), desc) : null,
    h('section', {}, h('div', { class: 'section-head' }, h('h2', {}, 'Теги'), tagControls), tagBox),
    shots, rels, releases,
  );
}

function communityBlock(d) {
  const list = (d.community || []).filter(l => settings().adult || !l.adult);
  if (!list.length) return null;
  const direct = list.filter(l => l.direct);
  const search = list.filter(l => !l.direct);
  return h('div', { class: 'where-group community' },
    h('h3', {}, 'Каталоги переводов и сообщества'),
    direct.map(l => h('div', { class: 'source' },
      h('a', { href: l.url, target: '_blank', rel: 'noopener noreferrer' }, `${l.name} — страница новеллы`),
      h('span', { class: 'langs' }, h('span', { class: 'lang' }, l.lang)),
      l.adult ? h('span', { class: 'mini ero' }, '18+') : null)),
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
    root.replaceChildren(
      h('div', { class: 'page-head' }, h('h1', {}, 'Мои полки')),
      h('div', { class: 'tabs' },
        h('a', { href: '#/my', class: current ? '' : 'on' }, `Все (${my.marks.length})`),
        Object.entries(MARKS).map(([k, m]) => h('a', { href: `#/my/${k}`, class: current === k ? 'on' : '' }, `${m.icon} ${m.label} (${counts[k] || 0})`))),
      ids.length
        ? resultGrid(offset => api.search({ ...baseRequest(), lang_mode: 'any', ero: 'show', ids, sort: 'title', offset, limit: 48 }))
        : h('p', { class: 'muted center' }, 'Здесь пусто. Откройте любую новеллу и нажмите «Хочу», «Читаю» и т.д.'),
    );
  } catch (e) {
    root.replaceChildren(errorBox(e));
  }
}

// ---------- settings ----------

export async function viewSettings(root) {
  const s = settings();
  const field = (label, control, hint) => h('label', { class: 'field' }, h('span', {}, label), control, hint ? h('small', { class: 'muted' }, hint) : null);
  const select = (key, options) => h('select', { onchange: e => { save({ [key]: e.target.value }); toast('Сохранено'); } },
    options.map(([v, l]) => h('option', { value: v, selected: String(s[key]) === String(v) }, l)));
  const check = (key, label) => h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: !!s[key], onchange: e => { save({ [key]: e.target.checked }); toast('Сохранено'); } }), label);
  const text = (key, ph) => {
    const i = h('input', { type: 'text', value: s[key], placeholder: ph });
    i.addEventListener('change', () => { save({ [key]: i.value.trim() }); toast('Сохранено'); });
    return i;
  };

  const srcBox = h('div', {}, spinner());
  const loadSources = async () => {
    try {
      const r = await api.sources();
      srcBox.replaceChildren(
        h('p', { class: 'muted small' }, 'Ссылки на эти сайты появляются в блоке «Где взять». Для сайтов с индексом даётся прямая ссылка на страницу новеллы, для остальных — поиск. Список можно расширить файлом sources.json в папке данных.'),
        h('ul', { class: 'src-list' }, r.sources.map(x => h('li', {},
          h('b', {}, x.name), ` (${x.lang}${x.adult ? ', 18+' : ''}) `,
          h('span', { class: 'muted small' }, x.kind === 'store_search' ? 'поиск по магазину, если на VNDB нет ссылок' : (x.description || '')),
          x.indexed ? h('div', { class: 'muted small' }, x.entries ? `индекс: ${x.entries} страниц, обновлён ${x.fetched_at}` : 'индекс ещё не загружен') : null))),
        h('button', { class: 'btn', onclick: async () => { await api.refreshSources(); toast('Обновляю индексы'); setTimeout(loadStatus, 800); } }, 'Обновить индексы'));
    } catch (e) { srcBox.replaceChildren(errorBox(e, loadSources)); }
  };
  loadSources();
  const dbBox = h('div', {}, spinner());
  const loadStatus = async () => {
    try {
      const st = await api.status();
      const btn = h('button', { class: 'btn primary', disabled: st.import.running, onclick: async () => {
        btn.disabled = true; await api.startImport(); toast('Обновление запущено'); setTimeout(loadStatus, 1000);
      } }, st.ready ? 'Обновить базу из VNDB' : 'Скачать базу VNDB');
      dbBox.replaceChildren(
        h('p', {}, st.ready ? `В базе ${st.vns.toLocaleString('ru')} новелл и ${st.tags} тегов. Дамп VNDB от ${st.dump_timestamp}, импорт ${st.imported_at} (UTC).` : 'База не загружена.'),
        btn, (st.import.running || st.import.error) ? jobPanel(st.import) : (st.import.kind ? h('div', { class: 'muted small' }, `Последняя задача: ${({ import: 'полная загрузка', sync: 'новые релизы', sources: 'индексы каталогов' })[st.import.kind] || st.import.kind} — ${st.import.finished ? 'готово' : '…'}`) : null));
      if (st.import.running) setTimeout(loadStatus, 1000);
      else if (st.ready && (st.import.log || []).includes('Ready')) api.resetTags();
    } catch (e) { dbBox.replaceChildren(errorBox(e, loadStatus)); }
  };
  loadStatus();

  root.append(
    h('div', { class: 'page-head' }, h('h1', {}, 'Настройки')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Язык'),
      field('Предпочитаемые языки', text('langs', 'en, ru'), 'Коды через запятую по приоритету: en, ru, uk, es, de… Используются для названий и сортировки ссылок.'),
      field('Какие новеллы показывать', select('langMode', [['readable', 'Есть на любом языке, кроме японского/китайского/корейского'], ['list', 'Только на языках из списка выше'], ['any', 'Все, даже только на японском']])),
      check('allowMtl', 'Учитывать машинные переводы')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Контент'),
      check('adult', 'Показывать 18+ (полки, теги, новеллы)'),
      field('Размывать обложки и скриншоты', select('blur', [['explicit', 'Только откровенные'], ['suggestive', 'Откровенные и пикантные'], ['off', 'Не размывать']])),
      field('Спойлеры в тегах', select('spoiler', [['0', 'Скрывать'], ['1', 'Показывать мелкие'], ['2', 'Показывать все']])),
      check('hideCancelled', 'Скрывать отменённые проекты')),
    h('section', { class: 'panel form' }, h('h2', {}, 'Вид'),
      field('Тема', select('theme', [['auto', 'Как в системе'], ['dark', 'Тёмная'], ['light', 'Светлая']]))),
    h('section', { class: 'panel form' }, h('h2', {}, 'База данных и обновления'), dbBox,
      field('Обновления', select('updatePolicy', [
        ['smart', 'Новые релизы — сами, полную загрузку — спрашивать'],
        ['auto', 'Всё автоматически (и полную загрузку тоже)'],
        ['manual', 'Только вручную']])),
      h('div', { class: 'toolbar' },
        h('button', { class: 'btn', onclick: async () => { const m = await import('./app.js'); m.checkUpdates({ force: true, interactive: true }); } }, 'Проверить обновления'),
        h('button', { class: 'btn', onclick: async () => { await api.sync(); toast('Подтягиваю новые релизы'); setTimeout(loadStatus, 800); } }, 'Подтянуть новые релизы'))),
    h('section', { class: 'panel form' }, h('h2', {}, 'Каталоги переводов и сообщества'), srcBox),
    IN_APP
      ? h('section', { class: 'panel form' }, h('h2', {}, 'Приложение'),
        h('p', {}, `VN Shelf для Android, версия ${APP_VERSION}. Новые версии публикуются в GitHub Releases.`),
        h('div', { class: 'toolbar' },
          h('button', { class: 'btn', onclick: async () => { const m = await import('./app.js'); m.checkAppUpdate({ interactive: true }); } }, 'Проверить версию приложения'),
          h('button', { class: 'btn', onclick: () => { save({ ...DEFAULTS }); toast('Настройки сброшены'); location.reload(); } }, 'Сбросить настройки')))
      : h('section', { class: 'panel form' }, h('h2', {}, 'Сервер'),
        field('Адрес сервера', text('apiBase', 'пусто = этот же сервер'), 'Нужно для мобильного приложения: например http://192.168.1.10:8080'),
        h('p', { class: 'muted small' }, 'Чтобы открыть с телефона: запустите на ПК «vndb-web serve --addr 0.0.0.0:8080» и откройте http://IP-компьютера:8080 в браузере телефона. ' +
          'Там же можно «Установить приложение» / «На экран Домой».'),
        h('button', { class: 'btn', onclick: () => { save({ ...DEFAULTS }); toast('Настройки сброшены'); location.reload(); } }, 'Сбросить настройки')),
  );
}
