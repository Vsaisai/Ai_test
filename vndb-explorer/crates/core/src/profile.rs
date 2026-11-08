//! Personal profile: reading statistics and achievements computed from "My lists" (marks) and
//! the catalog (tags, length, release year, rating). Nothing is stored except the marks themselves,
//! so achievements always match the lists.

use crate::catalog::{Catalog, Vn};
use serde::Serialize;

/// One personal mark: VN id and status ("wish", "playing", "finished", "dropped", "favorite").
pub struct Mark {
    pub vid: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Achievement {
    pub id: &'static str,
    pub icon: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub progress: u32,
    pub goal: u32,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreCount {
    pub tag: u32,
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub wish: u32,
    pub playing: u32,
    /// Finished + favourite (a favourite is a read VN too).
    pub read: u32,
    pub dropped: u32,
    pub favorite: u32,
    /// Sum of the average length of read VNs, in hours.
    pub hours: u32,
    /// Most frequent genres among read VNs.
    pub genres: Vec<GenreCount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub stats: Stats,
    pub achievements: Vec<Achievement>,
}

/// Genre tags (VNDB names) counted for "favourite genres" and the "all-rounder" achievement.
const GENRES: &[&str] = &[
    "Romance",
    "Comedy",
    "Drama",
    "Mystery",
    "Horror",
    "Fantasy",
    "Science Fiction",
    "Action",
    "Slice of Life",
    "Thriller",
];

fn has_tag(cat: &Catalog, v: &Vn, root: u32) -> bool {
    let set = cat.tree.subtree(root);
    v.tags
        .iter()
        .any(|t| !t.lie && t.rating >= 1.0 && t.spoiler == 0 && set.contains(t.tag))
}

fn tag_id(cat: &Catalog, name: &str) -> Option<u32> {
    cat.tree.resolve(name).ok()
}

pub fn compute(cat: &Catalog, marks: &[Mark], this_year: u32) -> Profile {
    let count = |s: &str| marks.iter().filter(|m| m.status == s).count() as u32;
    let read: Vec<&Vn> = marks
        .iter()
        .filter(|m| m.status == "finished" || m.status == "favorite")
        .filter_map(|m| cat.get(m.vid))
        .collect();
    let hours: u32 = read.iter().filter_map(|v| v.length_minutes).sum::<u32>() / 60;

    let mut genres: Vec<GenreCount> = GENRES
        .iter()
        .filter_map(|g| {
            let id = tag_id(cat, g)?;
            let n = read.iter().filter(|v| has_tag(cat, v, id)).count() as u32;
            Some(GenreCount {
                tag: id,
                name: g.to_string(),
                count: n,
            })
        })
        .filter(|g| g.count > 0)
        .collect();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));

    let with_tag = |name: &str| -> u32 {
        tag_id(cat, name).map_or(0, |id| {
            read.iter().filter(|v| has_tag(cat, v, id)).count() as u32
        })
    };
    let year = |v: &Vn| v.released / 10000;
    let n_read = read.len() as u32;
    let max_len = read
        .iter()
        .filter_map(|v| v.length_minutes)
        .max()
        .unwrap_or(0)
        / 60;
    let short = read
        .iter()
        .filter(|v| v.length_minutes.is_some_and(|m| m <= 120))
        .count() as u32;
    let classic = read
        .iter()
        .filter(|v| year(v) > 0 && year(v) < 2005)
        .count() as u32;
    let fresh = read
        .iter()
        .filter(|v| year(v) >= this_year.saturating_sub(1))
        .count() as u32;
    let masterpiece = read.iter().filter(|v| v.rating >= 8.5).count() as u32;
    let gem = read
        .iter()
        .filter(|v| v.votecount > 0 && v.votecount < 100)
        .count() as u32;
    let dropped = count("dropped");

    let a = |id, icon, title, description, progress: u32, goal: u32| Achievement {
        id,
        icon,
        title,
        description,
        progress: progress.min(goal),
        goal,
        done: progress >= goal,
    };
    let achievements = vec![
        a(
            "first",
            "📖",
            "Первая страница",
            "Прочитать первую новеллу",
            n_read,
            1,
        ),
        a("reader5", "📚", "Читатель", "Прочитать 5 новелл", n_read, 5),
        a(
            "reader25",
            "🏛️",
            "Книжный червь",
            "Прочитать 25 новелл",
            n_read,
            25,
        ),
        a(
            "reader100",
            "👑",
            "Легенда полки",
            "Прочитать 100 новелл",
            n_read,
            100,
        ),
        a(
            "hours100",
            "⏳",
            "Марафонец",
            "100 часов в прочитанных новеллах",
            hours,
            100,
        ),
        a(
            "hours500",
            "🕰️",
            "Время — лишь число",
            "500 часов в прочитанных новеллах",
            hours,
            500,
        ),
        a(
            "long",
            "🛤️",
            "Долгая дорога",
            "Прочитать новеллу длиной от 50 часов",
            max_len,
            50,
        ),
        a(
            "short",
            "🍬",
            "Перекус",
            "Прочитать 5 коротких новелл (до 2 часов)",
            short,
            5,
        ),
        a(
            "classic",
            "📜",
            "Классика",
            "Прочитать новеллу, вышедшую до 2005 года",
            classic,
            1,
        ),
        a(
            "fresh",
            "✨",
            "С пылу с жару",
            "Прочитать новеллу этого или прошлого года",
            fresh,
            1,
        ),
        a(
            "allrounder",
            "🎭",
            "Всеядный",
            "Прочитать новеллы 5 разных жанров",
            genres.len() as u32,
            5,
        ),
        a(
            "romance",
            "💞",
            "Романтик",
            "Прочитать 5 романтических новелл",
            with_tag("Romance"),
            5,
        ),
        a(
            "horror",
            "🕯️",
            "Стальные нервы",
            "Прочитать 3 хоррора",
            with_tag("Horror"),
            3,
        ),
        a(
            "mystery",
            "🔍",
            "Детектив",
            "Прочитать 3 детектива или загадки",
            with_tag("Mystery"),
            3,
        ),
        a(
            "scifi",
            "🚀",
            "Фантаст",
            "Прочитать 3 научно-фантастические новеллы",
            with_tag("Science Fiction"),
            3,
        ),
        a(
            "masterpiece",
            "🏆",
            "Шедевр",
            "Прочитать новеллу с рейтингом 8.5+",
            masterpiece,
            1,
        ),
        a(
            "gem",
            "💎",
            "Находка",
            "Прочитать малоизвестную новеллу (меньше 100 голосов)",
            gem,
            1,
        ),
        a(
            "favorite5",
            "❤️",
            "Ценитель",
            "Добавить 5 новелл в «Любимое»",
            count("favorite"),
            5,
        ),
        a(
            "wish20",
            "🔖",
            "Большие планы",
            "Отложить 20 новелл в «Хочу»",
            count("wish"),
            20,
        ),
        a(
            "finisher",
            "🎯",
            "Довожу до конца",
            "Прочитать 10 новелл и ни одной не бросить",
            if dropped == 0 { n_read } else { 0 },
            10,
        ),
    ];
    Profile {
        stats: Stats {
            wish: count("wish"),
            playing: count("playing"),
            read: n_read,
            dropped,
            favorite: count("favorite"),
            hours,
            genres: genres.into_iter().take(5).collect(),
        },
        achievements,
    }
}
