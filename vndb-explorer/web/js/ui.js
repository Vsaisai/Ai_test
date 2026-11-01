// Small DOM helpers and shared components.
import { img } from './api.js';
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

export const KIND = ['офиц.', 'фан-перевод', 'машинный'];

export function langBadges(langs, max = 4) {
  const shown = langs.slice(0, max);
  const wrap = h('span', { class: 'langs' },
    shown.map(l => h('span', { class: `lang k${l.kind}`, title: `${LANG_NAMES[l.lang] || l.lang}: ${KIND[l.kind]}` }, l.lang)));
  if (langs.length > max) wrap.append(h('span', { class: 'lang more' }, `+${langs.length - max}`));
  return wrap;
}

export function fmtLength(min) {
  if (!min) return '';
  if (min < 60) return `${min} мин`;
  const hr = Math.round(min / 60);
  return `~${hr} ч`;
}

export function shouldBlur(sexual) {
  const b = settings().blur;
  if (b === 'off') return false;
  if (b === 'suggestive') return sexual > 40;
  return sexual > 130;
}

export function cover(path, sexual = 0, alt = '', cls = 'cover') {
  if (!path) return h('div', { class: `${cls} nocover` }, h('span', {}, 'нет обложки'));
  const im = h('img', { src: img(path), alt, loading: 'lazy', decoding: 'async' });
  const box = h('div', { class: cls }, im);
  if (shouldBlur(sexual)) {
    box.classList.add('blurred');
    box.append(h('button', {
      class: 'reveal', type: 'button', title: 'Показать',
      onclick: e => { e.preventDefault(); e.stopPropagation(); box.classList.remove('blurred'); e.currentTarget.remove(); },
    }, '18+'));
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
  if (it.ero) badges.append(h('span', { class: 'badge ero' }, '18+'));
  if (it.freeware) badges.append(h('span', { class: 'badge free' }, 'free'));
  if (it.mark && MARKS[it.mark]) badges.append(h('span', { class: 'badge mark', title: MARKS[it.mark].label }, MARKS[it.mark].icon));
  return h('a', { class: 'card', href: `#/vn/${it.id}`, title: it.original && it.original !== it.title ? `${it.title}\n${it.original}` : it.title },
    h('div', { class: 'cover-wrap' }, cover(it.image, it.img_sexual, it.title), badges),
    h('div', { class: 'card-title' }, it.title),
    h('div', { class: 'card-meta' }, meta),
    langBadges(it.langs, 3),
  );
}

export function spinner(text = 'Загрузка…') {
  return h('div', { class: 'loading' }, h('span', { class: 'spin' }), text);
}

export function errorBox(e, retry) {
  return h('div', { class: 'error' }, h('p', {}, '⚠️ ', e.message || String(e)), retry && h('button', { class: 'btn', onclick: retry }, 'Повторить'));
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

export const ROOT_RU = { Theme: 'Тема', Character: 'Персонажи', Setting: 'Сеттинг', Plot: 'Сюжет', Style: 'Стиль' };
