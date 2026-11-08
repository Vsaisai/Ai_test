// Small DOM helpers and shared components.
import { img, api } from './api.js';
import { catRu } from './taxonomy.js';
import { humanError } from './errors.js';
import { settings } from './store.js';

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, v);
  }
  for (const c of children.flat(Infinity)) {
    if (c === null || c === undefined || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

export const LANG_NAMES = {
  en: 'English', ru: 'Русский', uk: 'Українська', ja: '日本語', zh: '中文', 'zh-Hans': '简体中文', 'zh-Hant': '繁體中文',
  ko: '한국어', es: 'Español', de: 'Deutsch', fr: 'Français', it: 'Italiano', 'pt-br': 'Português (BR)', 'pt-pt': 'Português',
  pl: 'Polski', cs: 'Čeština', vi: 'Tiếng Việt', id: 'Indonesia', tr: 'Türkçe', ar: 'العربية', th: 'ไทย', hu: 'Magyar',
};

export const PLATFORMS = {
  win: 'Windows', lin: 'Linux', mac: 'macOS', web: 'Браузер', and: 'Android', ios: 'iOS', swi: 'Switch', ps4: 'PS4',
  ps5: 'PS5', psv: 'PS Vita', psp: 'PSP', ps3: 'PS3', ps2: 'PS2', xb1: 'Xbox One', xbs: 'Xbox Series', n3d: '3DS',
  nds: 'DS', vnd: 'VNDS', dvd: 'DVD', mob: 'Мобильные (старые)', drc: 'Dreamcast', oth: 'Другое',
};

const KIND = ['офиц.', 'фан-перевод', 'машинный'];

export function langBadges(langs, max = 4) {
  const shown = langs.slice(0, max);
  const wrap = h('span', { class: 'langs' },
    shown.map(l => h('span', { class: `lang k${l.kind}`, title: `${LANG_NAMES[l.lang] || l.lang}: ${KIND[l.kind]}` }, l.lang)));
  if (langs.length > max) wrap.append(h('span', { class: 'lang more' }, `+${langs.length - max}`));
  return wrap;
}

/** Legend for language badges, one line per style. With `langs`, only the styles present are listed. */
export function langLegend(langs = null) {
  const rows = [[0, 'официальный перевод или оригинал'], [1, 'фанатский перевод'], [2, 'машинный перевод']]
    .filter(([k]) => !langs || langs.some(l => l.kind === k));
  if (!rows.length) return null;
  return h('ul', { class: 'lang-legend' }, rows.map(([k, text]) =>
    h('li', {}, h('span', { class: `lang k${k}` }, 'EN'), h('span', { class: 'muted small' }, `— ${text}`))));
}

export function fmtLength(min) {
  if (!min) return '';
  if (min < 60) return `${min} мин`;
  const hr = Math.round(min / 60);
  return `~${hr} ч`;
}

function shouldBlur(sexual) {
  const b = settings().blur;
  if (b === 'off') return false;
  if (b === 'suggestive') return sexual > 40;
  return sexual > 130;
}

/**
 * Picture with optional sharper source: VNDB thumbnails are small (covers ≤256 px, screenshots
 * ≈136 px), so on large or high-density screens the browser picks `full` through srcset.
 * opts: { full, sizes, thumbW, fullW }
 */
export function cover(path, sexual = 0, alt = '', cls = 'cover', opts = {}) {
  if (!path) return h('div', { class: `${cls} nocover` }, h('span', {}, 'нет обложки'));
  const attrs = { src: img(path), alt, loading: 'lazy', decoding: 'async' };
  if (opts.full && opts.full !== path) {
    attrs.srcset = `${img(path)} ${opts.thumbW || 256}w, ${img(opts.full)} ${opts.fullW || 800}w`;
    attrs.sizes = opts.sizes || '(max-width: 760px) 120px, 150px';
  }
  const box = h('div', { class: cls }, h('img', attrs));
  if (shouldBlur(sexual)) {
    box.classList.add('blurred');
    box.append(h('button', {
      class: 'reveal', type: 'button', title: 'Показать изображение', 'aria-label': 'Показать изображение',
      onclick: e => { e.preventDefault(); e.stopPropagation(); box.classList.remove('blurred'); e.currentTarget.remove(); },
    }, 'Показать'));
  }
  return box;
}

export const MARKS = {
  wish: { label: 'Хочу', icon: '🔖' },
  playing: { label: 'Читаю', icon: '▶️' },
  finished: { label: 'Прочитано', icon: '✅' },
  dropped: { label: 'Брошено', icon: '⏹️' },
  favorite: { label: 'Любимое', icon: '❤️' },
};

export function card(it, opts = {}) {
  const meta = [];
  if (it.rating) meta.push(h('span', { class: 'rating', title: `${it.votes} голосов` }, `★ ${it.rating.toFixed(1)}`));
  if (it.released) meta.push(h('span', {}, it.released.slice(0, 4)));
  if (it.length_minutes) meta.push(h('span', {}, fmtLength(it.length_minutes)));
  const badges = h('div', { class: 'badges' });
  if (opts.wantCount) {
    badges.append(h('span', { class: `badge ${it.all_wanted ? 'full' : ''}`, title: 'Совпало желательных тегов' },
      `${it.matched.length}/${opts.wantCount}`));
  }
  if (it.freeware) badges.append(h('span', { class: 'badge free' }, 'free'));
  if (it.mark && MARKS[it.mark]) badges.append(h('span', { class: 'badge mark', title: MARKS[it.mark].label }, MARKS[it.mark].icon));
  const href = `#/vn/${it.id}${opts.hl && opts.hl.length ? `?hl=${opts.hl.join(',')}` : ''}`;
  return h('a', { class: 'card', href, title: it.original && it.original !== it.title ? `${it.title}\n${it.original}` : it.title },
    h('div', { class: 'cover-wrap' }, cover(it.image, it.img_sexual, it.title, 'cover', { full: it.image_full, thumbW: 256, fullW: 600 }), badges),
    h('div', { class: 'card-title' }, it.title),
    h('div', { class: 'card-meta' }, meta),
    langBadges(it.langs, 2),
  );
}

export function spinner(text = 'Загрузка…') {
  return h('div', { class: 'loading' }, h('span', { class: 'spin' }), text);
}

export function errorBox(e, retry) {
  return h('div', { class: 'error' }, h('p', {}, '⚠️ ', humanError(e.message || String(e))), retry && h('button', { class: 'btn', onclick: retry }, 'Повторить'));
}

export function toast(text, ms = 2200) {
  const t = document.getElementById('toast');
  t.textContent = text;
  t.classList.add('show');
  clearTimeout(t._timer);
  t._timer = setTimeout(() => t.classList.remove('show'), ms);
}

export function debounce(fn, ms = 250) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}


// ---------- job progress (download / import / sync) ----------

const JOB_TITLES = { import: 'Загрузка базы VNDB', sync: 'Новые новеллы и релизы', sources: 'Индексы каталогов' };
let detailsOpen = false;

/** Compact progress line: "42% text" and a bar (null percent = unknown size, running stripe). */
export function progressLine(percent, text) {
  return h('div', { class: 'job compact' },
    h('div', { class: 'job-head' }, h('b', {}, percent === null ? '…' : `${Math.floor(percent)}%`), h('span', {}, ` ${text}`)),
    bar(percent));
}

function bar(percent, cls = '') {
  const indet = percent === null || percent === undefined;
  return h('div', { class: `bar ${cls} ${indet ? 'indet' : ''}`, role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': indet ? undefined : Math.round(percent) },
    h('i', { style: indet ? {} : { width: `${Math.max(0, Math.min(100, percent))}%` } }));
}

function amount(p) {
  const mb = b => (b / 1048576).toFixed(b >= 10485760 ? 0 : 1).replace('.', ',');
  if (p.unit === 'bytes') return p.total ? `${mb(p.done)} из ${mb(p.total)} МБ` : (p.done ? `${mb(p.done)} МБ` : '—');
  return p.total ? `${p.done} из ${p.total}` : (p.done ? String(p.done) : '—');
}

/** Detailed progress, like the "more details" view of a file copy dialog. */
export function jobPanel(job, { compact = false, link = null } = {}) {
  const p = job.progress;
  const title = JOB_TITLES[job.kind] || 'Обработка';
  if (job.error) {
    return h('div', { class: 'job error-job' },
      h('div', { class: 'job-head' }, h('b', {}, `${title}: ошибка`)),
      h('p', { class: 'small' }, humanError(job.error)),
      job.kind === 'import' ? h('p', { class: 'muted small' }, 'Нажмите кнопку ещё раз — скачивание продолжится с места обрыва.') : null);
  }
  if (!job.running) return h('div', { class: 'job' }, h('div', { class: 'job-head' }, h('b', {}, `${title}: готово`)));
  const overall = p ? p.overall_percent : 0;
  const stageLine = p ? `Этап ${p.step} из ${p.steps} · ${p.label}` : 'Подготовка…';
  if (compact) {
    return h('div', { class: 'job compact' },
      h('div', { class: 'job-head' }, h('b', {}, `${Math.floor(overall)}%`), h('span', {}, ` ${title} · ${p ? p.label : 'подготовка'}`),
        link ? h('a', { href: link, class: 'small' }, 'подробнее') : null),
      bar(overall),
      p && p.detail ? h('div', { class: 'muted small' }, p.detail) : null);
  }
  const rows = p ? [
    ['Этап', `${p.step} из ${p.steps}: ${p.label}`],
    ['Выполнено', `${amount(p)}${p.stage_percent !== null && p.stage_percent !== undefined ? ` (${p.stage_percent.toFixed(1).replace('.', ',')}%)` : ''}`],
    ['Скорость', p.speed > 0 ? (p.unit === 'bytes' ? `${(p.speed / 1048576).toFixed(1).replace('.', ',')} МБ/с` : `${p.speed.toFixed(0)} в сек.`) : '—'],
    ['Осталось (этап)', p.eta || '—'],
    ['Прошло всего', p.elapsed],
  ] : [];
  const details = h('details', { class: 'job-details', open: detailsOpen, ontoggle: e => { detailsOpen = e.target.open; } },
    h('summary', {}, 'Подробнее'),
    h('dl', { class: 'meta' }, rows.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])),
    h('pre', { class: 'log' }, (job.log || []).slice(-8).join('\n')));
  return h('div', { class: 'job' },
    h('div', { class: 'job-head' }, h('span', { class: 'job-pct' }, `${Math.floor(overall)}%`), h('b', {}, title)),
    bar(overall, 'big'),
    h('div', { class: 'job-stage' }, h('span', {}, stageLine), p && p.stage_percent !== null && p.stage_percent !== undefined ? h('span', { class: 'muted' }, `${Math.floor(p.stage_percent)}%`) : null),
    bar(p ? p.stage_percent : null, 'thin'),
    p && p.detail ? h('div', { class: 'job-detail' }, p.detail) : null,
    details);
}

/** Description parts from the server (text, links to tags / VNs / sites) -> DOM nodes.
 *  `[spoiler]…[/spoiler]` markers may span several parts: everything inside is hidden until tapped. */
export function richText(parts, { onNavigate } = {}) {
  const out = [];
  let spoiler = null;
  const add = node => { if (spoiler) spoiler.append(node); else out.push(node); };
  for (const x of parts || []) {
    if (x.tag || x.vn || x.url) {
      add(x.tag ? h('a', { href: `#/tag/${x.tag}`, onclick: onNavigate }, x.text)
        : x.vn ? h('a', { href: `#/vn/${x.vn}`, onclick: onNavigate }, x.text)
          : h('a', { href: x.url, target: '_blank', rel: 'noopener' }, x.text));
      continue;
    }
    x.text.split(/(\[\/?spoiler\])/i).forEach(piece => {
      if (/^\[spoiler\]$/i.test(piece)) {
        if (!spoiler) { spoiler = h('span', { class: 'spoiler', title: 'Спойлер — нажмите', onclick: e => e.currentTarget.classList.add('shown') }); out.push(spoiler); }
      } else if (/^\[\/spoiler\]$/i.test(piece)) {
        spoiler = null;
      } else if (piece) {
        add(piece);
      }
    });
  }
  return out;
}

/** Tag description from /api/tags/:id: text with links to other tags, plus example screenshots. */
export function tagDescription(t, { onNavigate, examples = true } = {}) {
  const p = h('p', { class: 'desc' }, richText(t.parts && t.parts.length ? t.parts : [{ text: t.description || '' }], { onNavigate }));
  const s = settings();
  const shown = (t.examples || []).filter(e => !e.nsfw || s.adultTags);
  if (!examples || !shown.length) return p;
  const imgs = shown.map((e, i) => ({ src: img(e.path), alt: `Пример ${i + 1}`, nsfw: e.nsfw }));
  const strip = h('div', { class: 'tag-examples' },
    h('span', { class: 'muted small' }, 'Как это выглядит:'),
    h('div', { class: 'tag-examples-row' }, imgs.map((im, i) => {
      const nsfwBlur = im.nsfw && s.blur !== 'off';
      return h('button', { type: 'button', class: `tag-example${nsfwBlur ? ' blurred' : ''}`, 'aria-label': im.alt,
        onclick: async ev => {
          ev.preventDefault(); ev.stopPropagation();
          const m = await import('./lightbox.js'); m.openLightbox(imgs, i);
        } }, h('img', { src: im.src, alt: im.alt, loading: 'lazy', decoding: 'async' }));
    })));
  return h('div', {}, p, strip);
}

const nameCache = new Map();

let pending = new Map(); // name -> [elements]
let flushTimer = null;
/** A text node-like <span> with the Russian name of a tag/category; untranslated names are
 *  machine-translated in one batched request and filled in when ready. */
export function ruLabel(name) {
  const el = h('span', {}, catRu(name));
  if (catRu(name) !== name || !settings().translate || !/[A-Za-z]/.test(name)) return el;
  if (nameCache.has(name)) { el.textContent = nameCache.get(name); el.title = name; return el; }
  if (!pending.has(name)) pending.set(name, []);
  pending.get(name).push(el);
  clearTimeout(flushTimer);
  flushTimer = setTimeout(async () => {
    const batch = pending; pending = new Map();
    const names = [...batch.keys()];
    for (let i = 0; i < names.length; i += 150) {
      const part = names.slice(i, i + 150);
      try {
        const r = await api.translate(part);
        part.forEach((n, j) => {
          const t = r.translations[j];
          if (!t) return;
          nameCache.set(n, t);
          batch.get(n).forEach(e => { e.textContent = t; e.title = n; });
        });
      } catch { /* offline: keep English */ }
    }
  }, 40);
  return el;
}
