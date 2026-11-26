// Personal page: avatar, name, reading stats, achievements; the settings are opened from here.
import { api } from './api.js';
import { h, spinner, errorBox, toast } from './ui.js';
import { tagLabel } from './taxonomy.js';

/** Square crop + resize on the device, so only a small JPEG is stored. */
async function resizeAvatar(file, size = 256) {
  const url = URL.createObjectURL(file);
  try {
    const img = await new Promise((ok, fail) => { const i = new Image(); i.onload = () => ok(i); i.onerror = fail; i.src = url; });
    const side = Math.min(img.naturalWidth, img.naturalHeight);
    const c = document.createElement('canvas');
    c.width = c.height = size;
    c.getContext('2d').drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, size, size);
    return c.toDataURL('image/jpeg', 0.85);
  } finally {
    URL.revokeObjectURL(url);
  }
}

function avatarNode(p) {
  if (p.avatar) return h('img', { src: p.avatar, alt: '' });
  const letter = (p.name || '').trim()[0];
  return h('span', { class: 'avatar-letter' }, letter ? letter.toUpperCase() : '📚');
}

export async function viewProfile(root) {
  root.append(spinner());
  let p;
  try { p = await api.profile(); } catch (e) { root.replaceChildren(errorBox(e)); return; }

  const pic = h('span', { class: 'avatar' }, avatarNode(p));
  let avatarButton, removeAvatar;
  const avatarBusy = busy => { avatarButton.disabled = busy; removeAvatar.disabled = busy; };
  const file = h('input', { type: 'file', accept: 'image/*', hidden: true, onchange: async e => {
    const f = e.target.files?.[0];
    if (!f) return;
    avatarBusy(true);
    try {
      const avatar = await resizeAvatar(f);
      await api.saveProfile({ avatar });
      p.avatar = avatar;
      pic.replaceChildren(avatarNode(p));
      removeAvatar.hidden = false;
      toast('Аватарка сохранена');
    } catch (err) { toast(err.message || 'Не удалось прочитать картинку'); }
    finally { file.value = ''; avatarBusy(false); }
  } });
  avatarButton = h('button', { class: 'avatar-btn', type: 'button', 'aria-label': 'Сменить аватарку', onclick: () => file.click() }, pic, h('span', { class: 'avatar-edit' }, '✎'));
  removeAvatar = h('button', { class: 'linkish small', type: 'button', hidden: !p.avatar, onclick: async () => {
    avatarBusy(true);
    try {
      await api.saveProfile({ avatar: '' });
      p.avatar = null;
      pic.replaceChildren(avatarNode(p));
      removeAvatar.hidden = true;
    } catch (err) { toast(err.message || 'Не удалось убрать аватарку'); }
    finally { avatarBusy(false); }
  } }, 'Убрать аватарку');
  const name = h('input', { type: 'text', class: 'profile-name', value: p.name || '', placeholder: 'Ваше имя', maxlength: 40, 'aria-label': 'Имя' });
  name.addEventListener('change', async () => {
    try { await api.saveProfile({ name: name.value }); p.name = name.value; pic.replaceChildren(avatarNode(p)); toast('Сохранено'); } catch (err) { toast(err.message); }
  });

  const s = p.stats;
  const stat = (n, label, href) => h('a', { class: 'stat', href }, h('b', {}, n.toLocaleString('ru')), h('span', {}, label));
  const done = p.achievements.filter(a => a.done);
  const todo = p.achievements.filter(a => !a.done).sort((a, b) => b.progress / b.goal - a.progress / a.goal);
  // Profile genre names are already provided by the server. Translate them directly:
  // loading the whole tag catalog here can otherwise delay the profile indefinitely.
  const genreName = g => tagLabel({ name: g.name });

  root.replaceChildren(
    h('section', { class: 'profile-head' },
      avatarButton,
      h('div', { class: 'profile-who' }, name,
        h('p', { class: 'muted small' }, done.length ? `Достижений: ${done.length} из ${p.achievements.length}` : 'Отмечайте новеллы — появятся достижения'),
        removeAvatar),
      file),
    h('div', { class: 'stats' },
      stat(s.read, 'прочитано', '#/my/finished'),
      stat(s.hours, 'часов', '#/my/finished'),
      stat(s.playing, 'читаю', '#/my/playing'),
      stat(s.wish, 'хочу', '#/my/wish')),
    ...(s.genres.length ? [h('p', { class: 'muted' }, 'Любимые жанры: ', s.genres.map(g => `${genreName(g)} (${g.count})`).join(', '))] : []),
    h('h2', {}, 'Достижения'),
    h('div', { class: 'achievements' }, [...done, ...todo].map(a => h('div', { class: `ach ${a.done ? 'done' : ''}`, title: a.description },
      h('span', { class: 'ach-icon' }, a.icon),
      h('div', { class: 'ach-text' }, h('b', {}, a.title), h('span', { class: 'muted small' }, a.description),
        a.done ? null : h('div', { class: 'ach-bar' }, h('i', { style: { width: `${Math.round(a.progress * 100 / a.goal)}%` } }), h('small', {}, `${a.progress} / ${a.goal}`)))))),
    h('a', { class: 'settings-link', href: '#/settings' }, h('span', {}, '⚙️'), h('span', {}, 'Настройки', h('small', { class: 'muted' }, 'язык, контент, база, обновления')), h('span', { class: 'muted' }, '›')),
  );
}
