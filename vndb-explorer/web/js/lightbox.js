// Full-screen image viewer: the whole picture fits the screen, swipe / arrows to browse,
// the Back button (or Esc) closes it instead of leaving the page.
import { h } from './ui.js';

let current = null;

function close(fromHistory) {
  if (!current) return;
  const { el, onKey } = current;
  current = null;
  document.removeEventListener('keydown', onKey);
  document.body.classList.remove('lb-open');
  el.remove();
  if (!fromHistory && history.state && history.state.lightbox) history.back();
}

window.addEventListener('popstate', () => close(true));

/** images: [{src, alt}] — src is a full URL. */
export function openLightbox(images, start = 0) {
  if (!images.length) return;
  close(true);
  let i = Math.max(0, Math.min(start, images.length - 1));
  const picture = h('img', { class: 'lb-img', alt: '', decoding: 'async' });
  const counter = h('span', { class: 'lb-count' });
  const spinner = h('span', { class: 'lb-wait spin' });
  const show = () => {
    spinner.hidden = false;
    picture.classList.add('loading');
    picture.onload = () => { spinner.hidden = true; picture.classList.remove('loading'); };
    picture.onerror = () => { spinner.hidden = true; };
    picture.src = images[i].src;
    picture.alt = images[i].alt || '';
    counter.textContent = images.length > 1 ? `${i + 1} / ${images.length}` : '';
    // warm up the neighbours
    for (const j of [i + 1, i - 1]) if (images[j]) new Image().src = images[j].src;
  };
  const go = d => { if (images.length > 1) { i = (i + d + images.length) % images.length; show(); } };
  const prev = h('button', { class: 'lb-nav lb-prev', type: 'button', 'aria-label': 'Назад', onclick: e => { e.stopPropagation(); go(-1); } }, '‹');
  const next = h('button', { class: 'lb-nav lb-next', type: 'button', 'aria-label': 'Дальше', onclick: e => { e.stopPropagation(); go(1); } }, '›');
  const closeBtn = h('button', { class: 'lb-close', type: 'button', 'aria-label': 'Закрыть', onclick: e => { e.stopPropagation(); close(false); } }, '×');
  const el = h('div', { class: 'lb', role: 'dialog', 'aria-modal': 'true', onclick: e => { if (e.target === el || e.target === stage) close(false); } });
  const stage = h('div', { class: 'lb-stage' }, picture, spinner);
  el.append(stage, h('div', { class: 'lb-bar' }, counter, closeBtn));
  if (images.length > 1) el.append(prev, next);

  // swipe
  let x0 = null, y0 = null;
  el.addEventListener('touchstart', e => { if (e.touches.length === 1) { x0 = e.touches[0].clientX; y0 = e.touches[0].clientY; } }, { passive: true });
  el.addEventListener('touchend', e => {
    if (x0 === null) return;
    const dx = e.changedTouches[0].clientX - x0, dy = e.changedTouches[0].clientY - y0;
    x0 = null;
    if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy)) go(dx < 0 ? 1 : -1);
    else if (dy > 90 && Math.abs(dy) > Math.abs(dx)) close(false);
  });
  const onKey = e => {
    if (e.key === 'Escape') close(false);
    else if (e.key === 'ArrowRight') go(1);
    else if (e.key === 'ArrowLeft') go(-1);
  };
  document.addEventListener('keydown', onKey);
  current = { el, onKey };
  history.pushState({ lightbox: true }, '');
  document.body.classList.add('lb-open');
  document.body.append(el);
  show();
  closeBtn.focus();
}
