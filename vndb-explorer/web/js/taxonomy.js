// Human-friendly tag catalog: curated groups with Russian labels on top of VNDB tags.
// Tags are referenced by their exact VNDB names; unknown names are skipped at runtime.

export const CURATED = [
  {
    id: 'genre', title: 'Жанр', tags: [
      ['Romance', 'Романтика'], ['Comedy', 'Комедия'], ['Drama', 'Драма'], ['Slice of Life', 'Повседневность'],
      ['Mystery', 'Мистика и загадки'], ['Detective Work', 'Детектив'], ['Horror', 'Ужасы'],
      ['Psychological Horror', 'Психологический хоррор'], ['Lovecraftian Horror', 'Лавкрафтовский хоррор'],
      ['Thriller', 'Триллер'], ['Fantasy', 'Фэнтези'], ['Dark Fantasy', 'Тёмное фэнтези'], ['Urban Fantasy', 'Городское фэнтези'],
      ['Science Fiction', 'Научная фантастика'], ['Cyberpunk', 'Киберпанк'], ['Action', 'Экшн'],
      ['Utsuge', 'Утсуге (грустное)'], ['Nakige', 'Накиге (до слёз)'], ['Parody', 'Пародия'], ['Sports', 'Спорт'],
      ['Educational', 'Обучающее'],
    ],
  },
  {
    id: 'setting', title: 'Сеттинг', tags: [
      ['School', 'Школа'], ['High School', 'Старшая школа'], ['Magic School', 'Школа магии'], ['Student Club', 'Школьный клуб'],
      ['Modern Day', 'Наше время'], ['Modern Day Japan', 'Современная Япония'], ['Countryside', 'Сельская местность'],
      ['Office', 'Офис'], ['The Holiday Season', 'Праздники'], ['Onsen', 'Онсэн'], ['Western-style Manor', 'Особняк'],
      ['Fictional World', 'Вымышленный мир'], ['Medieval Fantasy', 'Средневековое фэнтези'],
      ['Protagonist from a Different World', 'Попаданец (исекай)'], ['Alternate Dimensions', 'Другие измерения'],
      ['Space', 'Космос'], ['Post Apocalyptic Earth', 'Постапокалипсис'], ['Dystopia', 'Антиутопия'],
      ['Japanese Mythology', 'Японская мифология'],
    ],
  },
  {
    id: 'themes', title: 'Сюжет и темы', tags: [
      ['Time Travel', 'Путешествия во времени'], ['Time Loop', 'Временная петля'], ['Amnesia', 'Амнезия'],
      ['Magic', 'Магия'], ['Superpowers', 'Сверхспособности'], ['Vampire', 'Вампиры'], ['Demons', 'Демоны'],
      ['Deities', 'Боги'], ['Angels', 'Ангелы'], ['Ghost', 'Призраки'], ['Youkai', 'Ёкаи'], ['Werewolf', 'Оборотни'],
      ['Dragon', 'Драконы'], ['Monsters', 'Монстры'], ['Kemonomimi', 'Кемономими'], ['Robots', 'Роботы'],
      ['Extraterrestrial', 'Пришельцы'], ['War', 'Война'], ['Murder Mystery', 'Убийство'], ['Death Game', 'Смертельная игра'],
      ['Battle Royale', 'Королевская битва'], ['Survival Focus', 'Выживание'], ['Confinement', 'Заточение'],
      ['Revenge', 'Месть'], ['Friendship', 'Дружба'], ['Family', 'Семья'], ['Dream', 'Сны'], ['Afterlife', 'Загробный мир'],
      ['Madness', 'Безумие'], ['Mind Control', 'Контроль разума'], ['Philosophy', 'Философия'], ['Police', 'Полиция'],
      ['Japanese Idols', 'Айдолы'], ['Cosplay', 'Косплей'], ['Fairy Tale', 'Сказка'],
    ],
  },
  {
    id: 'protagonist', title: 'Главный герой', tags: [
      ['Male Protagonist', 'Мужчина'], ['Female Protagonist', 'Девушка'], ['Adult Protagonist', 'Взрослый'],
      ['High School Student Protagonist', 'Старшеклассник'], ['Multiple Protagonists', 'Несколько героев'],
      ['Protagonist with a Face', 'Лицо героя видно'], ['Protagonist with Voice Acting', 'Герой озвучен'],
      ['Antihero Protagonist', 'Антигерой'], ['Villainous Protagonist', 'Злодей'], ['Teacher Protagonist', 'Учитель'],
      ['Fighting Protagonist', 'Сражается'],
    ],
  },
  {
    id: 'romance', title: 'Романтика и отношения', tags: [
      ['Otome Game', 'Отомэ'], ['Girl x Girl Romance', 'Юри'], ['Boy x Boy Romance', 'BL'], ['Harem Ending', 'Гарем-концовка'],
      ['No Romance Plot', 'Без романтики'], ['Only a Single Heroine', 'Одна героиня'],
      ["Protagonist's Childhood Friend as a Heroine", 'Подруга детства'], ['Love Rivalry', 'Соперничество в любви'],
      ['Unrequited Love', 'Безответная любовь'], ['Student-Teacher Romance', 'Учитель и ученик'],
      ['Tsundere Heroine', 'Цундэрэ'], ['Kuudere Heroine', 'Кудэрэ'], ['Yandere Heroine', 'Яндэрэ (героиня)'],
      ['Yandere Hero', 'Яндэрэ (герой)'], ['Maid Heroine', 'Горничная'], ['Teacher Heroine', 'Учительница'],
      ['Tomboy Heroine', 'Пацанка'], ['Ojousama Heroine', 'Аристократка'],
    ],
  },
  {
    id: 'format', title: 'Формат и геймплей', tags: [
      ['Kinetic Novel', 'Без выборов (кинетическая)'], ['Multiple Endings', 'Несколько концовок'], ['One True End', 'Истинная концовка'],
      ['Branching Plot', 'Ветвящийся сюжет'], ['Linear Plot', 'Линейный сюжет'], ['Open Ending(s)', 'Открытый финал'],
      ['RPG', 'RPG'], ['Strategy Game', 'Стратегия'], ['Simulation Game', 'Симулятор'], ['Dating Simulation', 'Симулятор свиданий'],
      ['Puzzle Game', 'Головоломки'], ['Point-and-Click Adventure Game', 'Point-and-click'],
      ['Interactive Adventure Game', 'Адвенчура'], ['Escape Game', 'Квест-побег'], ['Fast Mini-games', 'Мини-игры'],
      ['ADV', 'ADV (текст внизу)'], ['NVL', 'NVL (текст на весь экран)'], ['Highly Animated Sprites', 'Анимированные персонажи'],
      ['No Sexual Content', 'Без откровенных сцен'],
    ],
  },
];

/** Shown only when "adult tags" are enabled in settings. */
export const ADULT = {
  id: 'adult', title: 'Для взрослых', tags: [
    ['Sexual Content', 'Есть откровенные сцены'], ['Nukige', 'Нукигэ'], ['Netorare', 'NTR'], ['Netori', 'Нэтори'],
    ['Lesbian Sex', 'Юри (откровенное)'], ['Male on Male Sex', 'Яой (откровенное)'], ['Group Sex', 'Групповое'],
    ['Female Domination', 'Фемдом'], ['BDSM', 'BDSM'], ['Sexual Corruption', 'Развращение'], ['Exhibitionism', 'Эксгибиционизм'],
    ['Sex With Monsters', 'С монстрами'], ['Sex with Tentacles', 'Тентакли'], ['Incest', 'Инцест'], ['Rape', 'Насилие'],
  ],
};

/** Russian names of VNDB tag categories (roots and second level). */
const CAT_RU = {
  Theme: 'Темы', Character: 'Персонажи', Setting: 'Сеттинг', Plot: 'Сюжет', Style: 'Стиль и геймплей',
  General: 'Общее', Action: 'Экшн', Comedy: 'Комедия', Drama: 'Драма', Educational: 'Обучающее', Fantasy: 'Фэнтези',
  Horror: 'Ужасы', Mystery: 'Мистика', 'Other Elements': 'Другие элементы', Romance: 'Романтика',
  'Science Fiction': 'Научная фантастика', 'Sexual Content': 'Для взрослых', Thriller: 'Триллер',
  Hero: 'Герой', Heroine: 'Героиня', 'Major Antagonist': 'Главный злодей', 'Misc. Character Tags': 'Разное о персонажах',
  'More Than Seven Love Interests': 'Больше семи романтических интересов', 'Non-Binary Love Interest': 'Небинарный романтический интерес',
  Protagonist: 'Протагонист', 'Support Characters': 'Второстепенные персонажи',
  Ending: 'Концовки', Routes: 'Маршруты', Type: 'Тип сюжета',
  Scene: 'Место действия', 'Time Period': 'Время действия', Universe: 'Мир',
  "April Fool's": 'Первоапрельское', Design: 'Интерфейс и устройство', 'Game Jam': 'Гейм-джем', Modifications: 'Моды',
  'Other Gameplay Elements': 'Геймплей', Presentation: 'Подача', 'Title Parody': 'Пародия на название',
  'Transformation Sequence(s)': 'Сцены трансформации',
};

const RU = new Map([...CURATED.flatMap(g => g.tags), ...ADULT.tags]);

/** Russian label of a tag (curated) or its VNDB name. */
export function tagLabel(t) {
  return (t && RU.get(t.name)) || (t && t.name) || '';
}

export function hasRu(t) {
  return !!(t && RU.has(t.name));
}

export function catRu(name) {
  return CAT_RU[name] || name;
}

/** Resolve a curated group against the loaded tag list: [{tag, label}] (missing names dropped). */
export function resolveGroup(group, tags) {
  if (!tags.byName) tags.byName = new Map(tags.list.map(t => [t.name, t]));
  return group.tags.map(([name, label]) => ({ tag: tags.byName.get(name), label })).filter(x => x.tag);
}
