// Turn technical error texts (network, disk, server) into short Russian messages.
// The original text stays in brackets when it adds detail.

// [pattern, message, keep the original text as detail]
const RULES = [
  [/database is not imported yet/i, 'База ещё не загружена: откройте «Настройки» и нажмите «Скачать базу VNDB».'],
  [/no space left|os error 28|disk is full|database or disk is full|недостаточно места/i, 'Не хватает места на устройстве. Освободите место и повторите.', true],
  [/os error 103|os error 104|connection reset|connection abort|broken pipe|connection closed|оборвал/i, 'Связь оборвалась. Повторите — загрузка продолжится с того же места.', true],
  [/timed out|timeout|deadline/i, 'Сервер долго не отвечает. Проверьте интернет и повторите.', true],
  [/dns|failed to lookup|name or service not known|no address associated|unknown host/i, 'Нет интернета (не удаётся найти сервер).', true],
  [/network is unreachable|no route to host|os error 101|connection refused|os error 111/i, 'Нет интернета или сервер недоступен.', true],
  [/certificate|tls|ssl|handshake/i, 'Не удалось установить защищённое соединение (VPN, прокси или неверное время на устройстве?).', true],
  [/no such vn/i, 'Такой новеллы нет в базе.'],
  [/bad id/i, 'Неверный номер.'],
  [/unknown tag/i, 'Такого тега нет.'],
  [/no shelf/i, 'Такой полки нет.'],
  [/cross-site request blocked/i, 'Запрос отклонён: открывайте приложение по его собственному адресу.'],
  [/HTTP 5\d\d/, 'Ошибка на стороне сервера. Повторите позже.'],
  [/HTTP 429|throttl/i, 'Слишком много запросов. Подождите минуту и повторите.'],
];

export function humanError(text) {
  const t = String(text || '').trim();
  if (!t) return 'Неизвестная ошибка';
  // already readable (our own Russian messages keep their numbers and hints)
  if (/[А-Яа-яЁё]/.test(t) && !/[A-Za-z]{5}/.test(t.replace(/VNDB|GitLab|dl\.vndb\.org|HTTP/g, ''))) return t[0].toUpperCase() + t.slice(1);
  for (const [re, msg, detail] of RULES) {
    if (re.test(t)) return detail ? `${msg} (${short(t)})` : msg;
  }
  return t;
}

function short(t) {
  return t.length > 140 ? `${t.slice(0, 140)}…` : t;
}
