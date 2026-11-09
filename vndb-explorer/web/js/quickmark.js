// Quick mark: hold a cover (or right-click it) to put the novel on a shelf without opening it.
import { api } from './api.js';
import { h, toast, MARKS } from './ui.js';

const HOLD_MS = 500;
let menu = null;
let suppressUntil = 0;

function close() {
  if (menu) { menu.remove(); menu = null; }
}

function paintBadge(card, status) {
  card.dataset.mark = status || '';
  const badges = card.querySelector('.badges');
  if (!badges) return;
  badges.querySelector('.badge.mark')?.remove();
  if (status && MARKS[status]) badges.append(h('span', { class: 'badge mark', title: MARKS[status].label }, MARKS[status].icon));
}

function open(card, x, y) {
  close();
  const id = card.dataset.vn;
  const cur = card.dataset.mark || '';
  const title = card.querySelector('.card-title')?.textContent || '';
  const pick = async status => {
    close();
    try {
      if (status) await api.quickMark(id, status); else await api.unmark(id);
      paintBadge(card, status);
      toast(status ? `На полке «${MARKS[status].label}»` : 'Убрано из моих полок');
      // "My shelves" counts and tabs depend on the marks: redraw that page
      if (location.hash.startsWith('#/my')) window.dispatchEvent(new HashChangeEvent('hashchange'));
    } catch (e) { toast(e.message); }
  };
  menu = h('div', { class: 'qm', role: 'menu', 'aria-label': `Полка для «${title}»` },
    h('div', { class: 'qm-title' }, title),
    Object.entries(MARKS).map(([k, m]) => h('button', { type: 'button', role: 'menuitem', class: k === cur ? 'on' : '', onclick: () => pick(k) }, `${m.icon} ${m.label}`)),
    cur ? h('button', { type: 'button', role: 'menuitem', class: 'rm', onclick: () => pick(null) }, '✕ Убрать с полки') : null);
  document.body.append(menu);
  // keep the menu on screen
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, innerHeight - r.height - 8))}px`;
  menu.querySelector('button')?.focus({ preventScroll: true });
}

export function installQuickMark() {
  let timer = null, sx = 0, sy = 0;
  const cancel = () => { clearTimeout(timer); timer = null; };
  const cardOf = t => t.closest('a.card[data-vn]');
  document.addEventListener('pointerdown', e => {
    if (menu && !menu.contains(e.target)) close();
    const card = cardOf(e.target);
    if (!card || e.button > 0 || e.target.closest('[data-tag], .reveal')) return;
    sx = e.clientX; sy = e.clientY;
    cancel();
    timer = setTimeout(() => {
      timer = null;
      suppressUntil = Date.now() + 800;
      if (navigator.vibrate) navigator.vibrate(15);
      open(card, sx, sy);
    }, HOLD_MS);
  });
  document.addEventListener('pointermove', e => { if (timer && Math.hypot(e.clientX - sx, e.clientY - sy) > 10) cancel(); });
  document.addEventListener('pointerup', cancel);
  document.addEventListener('pointercancel', cancel);
  // the click that ends a long press must not open the novel
  document.addEventListener('click', e => {
    if (Date.now() < suppressUntil && cardOf(e.target)) { e.preventDefault(); e.stopPropagation(); suppressUntil = 0; }
  }, true);
  document.addEventListener('contextmenu', e => {
    const card = cardOf(e.target);
    if (!card) return;
    e.preventDefault();
    open(card, e.clientX, e.clientY);
  });
  window.addEventListener('scroll', close, { passive: true });
  window.addEventListener('hashchange', close);
  document.addEventListener('keydown', e => { if (e.key === 'Escape') close(); });
}
