//! The short, polite live check against the real site (~4 + `games` requests, [`MIN_GAP`]
//! apart): dictionaries, the feed's first and last page, the forum's first and last page and a
//! few game pages. Everything it reads is saved through the normal store path.

use super::{cooldown_until_unix, F95Zone, ID};
use crate::external::{ExtGame, Provider};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CheckStep {
    pub name: String,
    pub ok: bool,
    pub details: Vec<String>,
}

/// A short, polite test against the real site: about 4 + `games` requests, 3 s apart.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LiveCheck {
    pub steps: Vec<CheckStep>,
    /// Releases in the feed, counted from its first and last page
    pub feed_releases: Option<usize>,
    /// Threads in the Games forum list, counted the same way (pinned ones left out)
    pub forum_threads: Option<usize>,
    pub ok: bool,
    pub stopped: Option<String>,
    pub at: String,
}

/// Exact size of a paged list from its first and last page.
pub(super) fn size(per_page: usize, last_page: u32, on_last: usize) -> usize {
    per_page * (last_page.saturating_sub(1) as usize) + on_last
}

pub fn live_check(
    conn: &rusqlite::Connection,
    games: usize,
    log: &dyn Fn(&str),
) -> anyhow::Result<LiveCheck> {
    let mut p = F95Zone::new();
    super::load_cooldown(&mut p, conn)?;
    if let Some(left) = cooldown_left() {
        log(&format!(
            "Сайт просил паузу: проверка подождёт ещё {} мин",
            left.div_ceil(60)
        ));
        std::thread::sleep(Duration::from_secs(left));
    }
    let mut out = LiveCheck {
        at: super::store::stamp(),
        ..Default::default()
    };
    let step = |out: &mut LiveCheck, name: &str, ok: bool, details: Vec<String>| {
        log(&format!(
            "{} {name}: {}",
            if ok { "✓" } else { "✗" },
            details.join("; ")
        ));
        out.steps.push(CheckStep {
            name: name.into(),
            ok,
            details,
        });
    };
    let stop = |out: &mut LiveCheck, e: &ProviderError| -> bool {
        if let ProviderError::Blocked(_) | ProviderError::RateLimited { .. } = e {
            out.stopped = Some(e.to_string());
            return true;
        }
        false
    };
    // 1. dictionaries
    match p.load_dictionaries() {
        Ok(()) => step(
            &mut out,
            "Словарь тегов",
            true,
            vec![
                format!(
                    "источник: {}",
                    if p.dictionary == "site" {
                        "сайт"
                    } else {
                        "встроенный снимок"
                    }
                ),
                format!("тегов: {}, префиксов: {}", p.tags.len(), p.prefixes.len()),
            ],
        ),
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Словарь тегов",
                false,
                vec![e.to_string(), "используется встроенный снимок тегов".into()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    // 2-3. the feed: first and last page give its exact size
    let mut sample: Vec<ExtGame> = Vec::new();
    match p.list(1) {
        Ok(first) => {
            let total = first.total_pages.unwrap_or(1);
            let mut d = vec![
                format!("игр на странице: {}, страниц: {}", first.games.len(), total),
                format!(
                    "с тегами: {}, со статусом: {}, нечитаемых записей: {}",
                    first.games.iter().filter(|g| !g.tags.is_empty()).count(),
                    first.games.iter().filter(|g| !g.status.is_empty()).count(),
                    first.bad_items
                ),
            ];
            d.extend(
                first
                    .games
                    .iter()
                    .take(3)
                    .map(|g| format!("«{}» — {}, {} тегов", g.title, g.developer, g.tags.len())),
            );
            let ok = !first.games.is_empty() && first.games.iter().any(|g| !g.tags.is_empty());
            step(&mut out, "Лента: первая страница", ok, d);
            for g in &first.games {
                super::store::upsert_listed(conn, g, "feed")?;
            }
            sample = first.games.clone();
            if total > 1 {
                match p.list(total) {
                    Ok(last) => {
                        // the page size the site really uses, not the one asked for
                        let n = size(first.games.len(), total, last.games.len());
                        out.feed_releases = Some(n);
                        step(
                            &mut out,
                            "Лента: последняя страница",
                            true,
                            vec![format!(
                                "на последней странице {}, всего в ленте {n}",
                                last.games.len()
                            )],
                        );
                    }
                    Err(e) => {
                        let fatal = stop(&mut out, &e);
                        step(
                            &mut out,
                            "Лента: последняя страница",
                            false,
                            vec![e.to_string()],
                        );
                        if fatal {
                            return finish(conn, out);
                        }
                    }
                }
            } else {
                out.feed_releases = Some(first.games.len());
            }
        }
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Лента: первая страница",
                false,
                vec![e.to_string()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    // 4-5. the Games forum list: the site's own index of every game thread
    match p.index_page(1) {
        Ok(first) => {
            let per = first.games.len();
            let last_page = first.last_page.unwrap_or(1);
            let mut d = vec![format!(
                "тем на странице: {per} (+ закреплённых служебных: {}), страниц: {last_page}",
                first.service
            )];
            d.extend(first.games.iter().take(3).map(|g| {
                format!(
                    "«{}» — {}",
                    g.title,
                    if g.developer.is_empty() {
                        "разработчик не указан"
                    } else {
                        &g.developer
                    }
                )
            }));
            step(&mut out, "Форум Games: первая страница", per > 0, d);
            if last_page > 1 && per > 0 {
                match p.index_page(last_page) {
                    Ok(last) => {
                        let n = size(per, last_page, last.games.len());
                        out.forum_threads = Some(n);
                        step(
                            &mut out,
                            "Форум Games: последняя страница",
                            true,
                            vec![format!(
                                "на последней странице {}, всего тем ≈ {n}",
                                last.games.len()
                            )],
                        );
                    }
                    Err(e) => {
                        let fatal = stop(&mut out, &e);
                        step(
                            &mut out,
                            "Форум Games: последняя страница",
                            false,
                            vec![e.to_string()],
                        );
                        if fatal {
                            return finish(conn, out);
                        }
                    }
                }
            } else {
                out.forum_threads = Some(per);
            }
        }
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Форум Games: первая страница",
                false,
                vec![e.to_string()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    if let (Some(f), Some(t)) = (out.feed_releases, out.forum_threads) {
        let diff = t as i64 - f as i64;
        step(
            &mut out,
            "Сравнение ленты и форума",
            diff <= 0,
            vec![
                format!("в ленте {f}, в форуме {t}"),
                if diff > 0 {
                    format!("в форуме на {diff} тем больше: лента неполная, нужна сверка с форумом")
                } else {
                    "в ленте не меньше тем, чем в форуме".into()
                },
            ],
        );
    }
    // 6. a few game pages
    for g in sample.iter().take(games) {
        match p.details(&g.ext_id) {
            Ok(d) => {
                let change = super::store::save_details(conn, &d)?;
                let ok = !d.title.is_empty() && !d.tags.is_empty() && d.description.is_some();
                step(
                    &mut out,
                    &format!("Страница игры {}", g.ext_id),
                    ok,
                    vec![
                        format!(
                            "«{}», разработчик: {}",
                            d.title,
                            if d.developer.is_empty() {
                                "—"
                            } else {
                                &d.developer
                            }
                        ),
                        format!(
                            "тегов: {}, описание: {} знаков, обложка: {}",
                            d.tags.len(),
                            d.description.as_ref().map_or(0, |x| x.chars().count()),
                            if d.cover.is_some() {
                                "есть"
                            } else {
                                "нет"
                            }
                        ),
                        format!(
                            "официальные ссылки: {}",
                            if d.links.is_empty() {
                                "нет".to_string()
                            } else {
                                d.links
                                    .iter()
                                    .map(|l| l.site.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        ),
                        if let super::store::Change::Hidden(r) = change {
                            format!("скрыта правилами ({r})")
                        } else {
                            "сохранена".into()
                        },
                    ],
                );
            }
            Err(e) => {
                let fatal = stop(&mut out, &e);
                step(
                    &mut out,
                    &format!("Страница игры {}", g.ext_id),
                    false,
                    vec![e.to_string()],
                );
                if fatal {
                    break;
                }
            }
        }
    }
    finish(conn, out)
}

fn finish(conn: &rusqlite::Connection, mut out: LiveCheck) -> anyhow::Result<LiveCheck> {
    if let Some(t) = cooldown_until_unix() {
        super::store::set_meta(conn, ID, "cooldown_until", &t.to_string())?;
    }
    out.ok = out.stopped.is_none() && !out.steps.is_empty() && out.steps.iter().all(|s| s.ok);
    super::store::set_meta(conn, ID, "last_check", &serde_json::to_string(&out)?)?;
    Ok(out)
}
