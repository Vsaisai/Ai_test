// Filter sheet (the funnel in the top bar): genres, tags, where to play, game language, more filters
// and collections. It edits a copy of the current search and opens the results with "Show N".
import { api } from './api.js';
import { h, debounce, PLATFORMS } from './ui.js';
import { settings } from './store.js';
import { CURATED, tagLabel, resolveGroup } from './taxonomy.js';
import { readSearch, writeSearch, searchRequest, tagPicker, sortSelect } from './views.js';

const MODES = ['want', 'must', 'exclude'];
const NEXT = { undefined: 'want', want: 'must', must: 'exclude', exclude: undefined };
const ICON = { want: '~', must: '!', exclude: '−' };
let open = null;

/** Close the sheet; with `to`, go there in place of the sheet's history entry (no async back()). */
export function closeFilters(to = null) {
  if (!open) return;
  open.remove();
  open = null;
  document.body.classList.remove('sheet-open');
  if (to) location.replace(to);
  else if (history.state?.sheet) history.back();
}
window.addEventListener('popstate', () => { if (open) { open.remove(); open = null; document.body.classList.remove('sheet-open'); } });

export async function openFilters() {
  if (open) { closeFilters(); return; }
  const [path, query = ''] = location.hash.slice(1).split('?');
  const st = readSearch(new URLSearchParams(path === '/search' ? query : ''));
  const tags = await api.tags().catch(() => null);
  if (!tags) return;
  // URL tag refs may be names: keep ids only
  for (const k of MODES) st[k] = st[k].map(x => Number(String(x).replace(/^g/, ''))).filter(n => tags.byId.has(n));

  const modeOf = id => MODES.find(k => st[k].includes(id));
  const setMode = (id, m) => { for (const k of MODES) st[k] = st[k].filter(x => x !== id); if (m) st[m].push(id); };
  const showBtn = h('button', { class: 'btn primary', type: 'button', onclick: () => closeFilters(writeSearch(st)) }, 'Показать');
  const count = debounce(async () => {
    try {
      const r = await api.search({ ...searchRequest(st), limit: 1 });
      showBtn.textContent = `Показать ${r.total.toLocaleString('ru')}`;
    } catch { showBtn.textContent = 'Показать'; }
  }, 250);

  // genres: tap cycles  → желательно → обязательно → исключить →
  const genreChip = ({ tag, label }) => {
    const b = h('button', { type: 'button', class: 'tchip' }, h('span', { class: 'tchip-mode' }), h('span', { class: 'tchip-label' }, label));
    const paint = () => { const m = modeOf(tag.id); if (m) b.dataset.mode = m; else b.removeAttribute('data-mode'); b.firstChild.textContent = m ? ICON[m] : ''; };
    b.onclick = () => { setMode(tag.id, NEXT[modeOf(tag.id)]); paint(); renderChosen(); count(); };
    b.dataset.tag = tag.id;
    b._paint = paint;
    paint();
    return b;
  };
  const genres = h('div', { class: 'tchips' }, resolveGroup(CURATED.find(g => g.id === 'genre'), tags).map(genreChip));

  // other tags: chosen chips + search field + whole catalog
  const chosen = h('div', { class: 'chips' });
  const genreIds = new Set(resolveGroup(CURATED.find(g => g.id === 'genre'), tags).map(x => x.tag.id));
  const renderChosen = () => {
    chosen.replaceChildren(...MODES.flatMap(k => st[k].filter(id => !genreIds.has(id)).map(id => {
      const t = tags.byId.get(id);
      return h('span', { class: `chip ${k}` },
        h('button', { type: 'button', class: 'chip-mode', onclick: () => { setMode(id, { want: 'must', must: 'exclude', exclude: 'want' }[k]); renderChosen(); count(); } }, ICON[k]),
        t ? tagLabel(t) : `g${id}`,
        h('button', { type: 'button', class: 'chip-x', 'aria-label': 'Убрать', onclick: () => { setMode(id, null); renderChosen(); count(); } }, '×'));
    })));
  };
  renderChosen();
  const picker = tagPicker(tags, (t, mode) => { setMode(t.id, mode); renderChosen(); genres.querySelectorAll('[data-tag]').forEach(b => b._paint()); count(); }, 'want');

  const choice = (key, options, reset) => h('div', { class: 'choice' }, options.map(([v, l]) => h('button', {
    type: 'button', class: v === (st[key] || options[0][0]) ? 'on' : '',
    onclick: e => { st[key] = v; if (reset) reset(); e.currentTarget.parentNode.querySelectorAll('button').forEach(x => x.classList.toggle('on', x === e.currentTarget)); count(); },
  }, l)));
  const num = (k, ph) => { const i = h('input', { type: 'number', min: 0, value: st[k], placeholder: ph }); i.oninput = () => { st[k] = i.value; count(); }; return i; };
  const plat = h('div', { class: 'checks' }, ['win', 'lin', 'mac', 'and', 'ios', 'web', 'swi', 'ps5', 'ps4', 'psv', 'xbs', 'xb1', 'n3d'].map(p => h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: st.plat.includes(p), onchange: e => { st.plat = e.target.checked ? [...st.plat, p] : st.plat.filter(x => x !== p); count(); } }), PLATFORMS[p])));

  const collections = h('div', { class: 'pills' });
  api.shelvesConfig().then(cfg => {
    const adult = !!settings().adultTags;
    collections.replaceChildren(...cfg.sections.filter(sec => adult || !sec.adult).flatMap(sec => sec.shelves.filter(sh => adult || !sh.adult)
      .map(sh => h('a', { class: 'pill', href: `#/shelf/${sh.id}`, onclick: e => { e.preventDefault(); closeFilters(`#/shelf/${sh.id}`); } }, sh.title))));
  }).catch(() => {});

  const section = (title, ...body) => h('section', { class: 'sheet-sec' }, h('h3', {}, title), ...body);
  const panel = h('div', { class: 'sheet', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Фильтры' },
    h('div', { class: 'sheet-head' }, h('b', {}, 'Фильтры'), h('button', { class: 'linkish', type: 'button', onclick: closeFilters, 'aria-label': 'Закрыть' }, '✕')),
    h('div', { class: 'sheet-body' },
      section('Жанр', h('p', { class: 'muted small' }, 'Касание: ~ желательно → ! обязательно → − исключить.'), genres),
      section('Подборки', collections),
      section('Теги', picker, chosen, h('a', { href: '#/tags', onclick: e => { e.preventDefault(); closeFilters(`#/tags?${writeSearch(st).split('?')[1] || ''}`); } }, 'Весь каталог тегов →')),
      section('Где играть', choice('dev', [['any', 'Везде'], ['pc', '💻 ПК'], ['phone', '📱 Телефон'], ['console', '🎮 Консоли']], () => { st.plat = []; })),
      section('Язык игры', choice('lang', [['', 'Есть не на японском'], ['ru', 'Русский'], ['en', 'Английский'], ['any', 'Любой']])),
      h('details', { class: 'sheet-sec' }, h('summary', {}, h('h3', {}, 'Ещё фильтры')),
        h('div', { class: 'filter-grid' },
          h('div', { class: 'field' }, h('span', {}, 'Длина, минут'), h('div', { class: 'pair' }, num('lmin', 'от'), num('lmax', 'до'))),
          h('div', { class: 'field' }, h('span', {}, 'Год выхода'), h('div', { class: 'pair' }, num('yfrom', 'с'), num('yto', 'по'))),
          h('div', { class: 'field' }, h('span', {}, 'Мин. голосов'), num('votes', '0')),
          h('div', { class: 'field' }, h('span', {}, 'Мин. рейтинг (1–10)'), num('rating', '0')),
          h('div', { class: 'field' }, h('span', {}, 'Мин. желательных тегов'), num('mm', '0')),
          h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: st.free, onchange: e => { st.free = e.target.checked; count(); } }), 'Только бесплатные'),
          h('div', { class: 'field' }, h('span', {}, 'Сортировка'), sortSelect(st.sort, v => { st.sort = v; })),
          h('div', { class: 'field full' }, h('span', {}, 'Точные платформы'), plat))),
      ),
    h('div', { class: 'sheet-foot' },
      h('button', { class: 'btn', type: 'button', onclick: () => closeFilters('#/search') }, 'Сбросить'),
      showBtn));
  const overlay = h('div', { class: 'sheet-wrap', onclick: e => { if (e.target === overlay) closeFilters(); } }, panel);
  open = overlay;
  history.pushState({ sheet: true }, '');
  document.body.classList.add('sheet-open');
  document.body.append(overlay);
  count();
}
