// Long press (or right click) on any element with data-tag="<id>" shows what the tag means.
// The card appears above the finger (or below if there is no room) so the finger does not cover it.
import { api } from './api.js';
import { h, tagDescription, ruLabel } from './ui.js';
import { settings } from './store.js';
import { tagLabel } from './taxonomy.js';

const HOLD_MS = 450;
const cache = new Map();
let tip = null;
let suppressUntil = 0;

function close() {
  if (tip) { tip.remove(); tip = null; }
}

async function info(id) {
  if (!cache.has(id)) cache.set(id, api.tag(id).catch(e => { cache.delete(id); throw e; }));
  return cache.get(id);
}

function place(el, x, y) {
  const vw = document.documentElement.clientWidth;
  const vh = window.innerHeight;
  const w = Math.min(340, vw - 16);
  el.style.width = `${w}px`;
  el.style.left = `${Math.max(8, Math.min(x - w / 2, vw - w - 8))}px`;
  const hgt = el.offsetHeight;
  const finger = 34; // keep clear of the fingertip
  let top = y - finger - hgt;
  if (top < 8) top = Math.min(y + finger, vh - hgt - 8);
  el.style.top = `${Math.max(8, top)}px`;
}

async function show(id, x, y) {
  close();
  const body = h('div', { class: 'tagtip-body' }, h('div', { class: 'muted small' }, 'Загрузка…'));
  const el = h('div', { class: 'tagtip', role: 'tooltip' }, body);
  tip = el;
  document.body.append(el);
  place(el, x, y);
  let t;
  try { t = await info(id); } catch (e) { body.replaceChildren(h('div', {}, e.message)); place(el, x, y); return; }
  if (tip !== el) return;
  const label = tagLabel(t);
  const desc = h('div', { class: 'tagtip-desc' });
  const original = t.description || '';
  const showOriginal = () => {
    desc.replaceChildren(original || t.examples?.length ? tagDescription(t, { onNavigate: close }) : 'Описания нет.');
    desc.querySelectorAll('img').forEach(im => im.addEventListener('load', () => { if (tip === el) place(el, x, y); }));
  };
  showOriginal();
  const tr = h('button', { class: 'linkish small', type: 'button' }, 'Перевести');
  let translated = false;
  const toggle = async () => {
    if (translated) { showOriginal(); tr.textContent = 'Перевести'; translated = false; place(el, x, y); return; }
    tr.textContent = 'Перевожу…';
    try {
      const r = await api.translate([original]);
      const p = desc.querySelector('p.desc');
      if (p) p.textContent = r.translations[0]; else desc.textContent = r.translations[0];
      tr.textContent = 'Оригинал'; translated = true;
    } catch (err) { tr.textContent = 'Нет связи'; }
    place(el, x, y);
  };
  body.replaceChildren(
    h('div', { class: 'tagtip-title' }, label === t.name ? ruLabel(t.name) : label, label !== t.name || settings().translate ? h('span', { class: 'muted small' }, ` · ${t.name}`) : null),
    h('div', { class: 'muted small' }, t.path_names.slice(0, -1).flatMap((n, i) => i ? [' › ', ruLabel(n)] : [ruLabel(n)]), t.path_names.length > 1 ? '' : '—', ` · ${t.vns} новелл`),
    desc,
    h('div', { class: 'tagtip-actions' },
      original ? tr : null,
      h('a', { href: `#/tag/${t.id}`, class: 'small', onclick: close }, 'Открыть тег →')));
  place(el, x, y);
  tr.onclick = e => { e.stopPropagation(); toggle(); };
  if (settings().translate && original) toggle();
}

export function installTagTips() {
  let timer = null, sx = 0, sy = 0;
  const cancel = () => { clearTimeout(timer); timer = null; };
  document.addEventListener('pointerdown', e => {
    if (tip && !tip.contains(e.target)) close();
    const target = e.target.closest('[data-tag]');
    if (!target || e.button > 0) return;
    sx = e.clientX; sy = e.clientY;
    cancel();
    timer = setTimeout(() => {
      timer = null;
      suppressUntil = Date.now() + 800;
      if (navigator.vibrate) navigator.vibrate(15);
      show(Number(target.dataset.tag), sx, sy);
    }, HOLD_MS);
  });
  document.addEventListener('pointermove', e => { if (timer && Math.hypot(e.clientX - sx, e.clientY - sy) > 10) cancel(); });
  document.addEventListener('pointerup', cancel);
  document.addEventListener('pointercancel', cancel);
  // the click that ends a long press must not toggle / navigate
  document.addEventListener('click', e => {
    if (Date.now() < suppressUntil && e.isTrusted && e.target.closest('[data-tag]')) { e.preventDefault(); e.stopPropagation(); suppressUntil = 0; }
  }, true);
  document.addEventListener('contextmenu', e => {
    const target = e.target.closest('[data-tag]');
    if (!target) return;
    e.preventDefault();
    if (!tip) show(Number(target.dataset.tag), e.clientX, e.clientY);
  });
  window.addEventListener('scroll', close, { passive: true });
  window.addEventListener('hashchange', close);
  document.addEventListener('keydown', e => { if (e.key === 'Escape') close(); });
}
