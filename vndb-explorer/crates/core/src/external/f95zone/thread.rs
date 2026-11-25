//! Reading the public XenForo pages: one game thread ([`game_page`]) and one page of the
//! Games forum thread list ([`forum_page`]). The low-level HTML helpers live in [`super::html`].

use super::{check_response, engine_of, html, status_of, F95Zone, DESCRIPTION_MAX, GAMES_FORUM, ID};
use crate::external::{official_site, ExtGame, OfficialLink, ProviderError};
use std::collections::HashSet;

// ---------- helpers for the first post ----------

/// `Name: value` on its own line in the first post.
fn field(text: &str, names: &[&str]) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    // earlier names win ("Developer/Publisher" before "Developer")
    for n in names {
        for (i, l) in lines.iter().enumerate() {
            let (Some(head), Some(tail)) = (l.get(..n.len()), l.get(n.len()..)) else {
                continue;
            };
            if !head.eq_ignore_ascii_case(n) {
                continue;
            }
            let next = || lines[i + 1..].iter().find(|x| !x.is_empty()).copied();
            let tail = tail.trim_start();
            let value = if let Some(v) = tail.strip_prefix(':') {
                // "Name: value", or the value on the next line
                let v = v.trim();
                if v.is_empty() {
                    next().unwrap_or("")
                } else {
                    v
                }
            } else if tail.is_empty() {
                // "Name" alone, ": value" on the next line
                match next().and_then(|x| x.strip_prefix(':')) {
                    Some(v) => v.trim(),
                    None => continue,
                }
            } else {
                continue;
            };
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Names the developer line goes by in first posts.
const DEVELOPER_FIELDS: &[&str] = &[
    "Developer/Publisher",
    "Developer & Publisher",
    "Developer / Publisher",
    "Original Developer",
    "Developers",
    "Developer",
    "Publisher",
    "Artist",
];

/// "Acme Games - Patreon - Discord" -> "Acme Games": the name without the link labels that
/// follow it on the same line.
fn clean_developer(raw: &str) -> String {
    const SEPARATORS: &[&str] = &[" - ", " – ", " — ", " | ", " / ", ", ", " (", " ["];
    const LINK_WORDS: &[&str] = &[
        "patreon",
        "subscribestar",
        "itch.io",
        "itch",
        "discord",
        "twitter",
        "website",
        "steam",
        "blog",
        "tumblr",
        "pixiv",
        "fanbox",
        "ko-fi",
        "kofi",
        "gumroad",
        "buymeacoffee",
        "dlsite",
        "youtube",
        "instagram",
        "facebook",
        "trello",
        "wiki",
    ];
    let mut name = raw.trim();
    for sep in SEPARATORS {
        if let Some(i) = name.find(sep) {
            // keep the text when the name itself would be lost ("- Patreon")
            if !name[..i].trim().is_empty() {
                name = name[..i].trim();
            }
        }
    }
    // link labels glued to the name with spaces only: "Acme Patreon Discord"
    let mut words: Vec<&str> = name.split_whitespace().collect();
    while words.len() > 1
        && words
            .last()
            .is_some_and(|w| LINK_WORDS.contains(&w.to_lowercase().as_str()))
    {
        words.pop();
    }
    words
        .join(" ")
        .trim_matches(|c: char| "-–—|/\\:,()[]{} ".contains(c))
        .to_string()
}

/// Free text after "Overview:" up to the next `Header:` line.
fn overview(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let i = ["overview", "story", "synopsis", "description"]
        .iter()
        .filter_map(|k| {
            let i = lower.find(&format!("{k}:"))?;
            Some(i + k.len() + 1)
        })
        .min()?;
    let mut out: Vec<&str> = Vec::new();
    for line in text[i..].lines() {
        let l = line.trim();
        let is_header = l.len() < 40 && l.ends_with(':') | l.contains(": ") && {
            let head = l.split(':').next().unwrap_or("");
            !head.is_empty()
                && head.len() < 30
                && head.chars().next().is_some_and(|c| c.is_uppercase())
                && head
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '/')
        };
        if !out.is_empty() && (is_header || l.eq_ignore_ascii_case("download")) {
            break;
        }
        if !l.is_empty() || !out.is_empty() {
            out.push(l);
        }
    }
    let s = out.join("\n").trim().to_string();
    if s.is_empty() {
        return None;
    }
    // the whole public overview, read only when the user opens the game (like a browser);
    // a safety limit against pages that swallow the rest of the post
    Some(excerpt(&s, DESCRIPTION_MAX))
}

fn excerpt(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let end = cut
        .rfind(['.', '!', '?'])
        .filter(|&i| i > max / 2)
        .map(|i| i + 1);
    format!("{}…", cut[..end.unwrap_or(cut.len())].trim_end())
}

/// Where the download section starts in the first post (everything after it is ignored).
fn download_section(body_lower: &str) -> usize {
    [
        ">download<",
        ">download:",
        ">downloads<",
        "<b>download",
        "download</b>",
        "download</span>",
    ]
    .iter()
    .filter_map(|m| body_lower.find(m))
    .min()
    .unwrap_or(body_lower.len())
}

/// Split "Name [v0.5] [Developer]" into name and bracket parts.
fn split_title(t: &str) -> (String, Vec<String>) {
    let mut brackets: Vec<String> = Vec::new();
    let mut name = t.trim().to_string();
    while name.ends_with(']') {
        let Some(i) = name.rfind('[') else { break };
        brackets.insert(0, name[i + 1..name.len() - 1].trim().to_string());
        name = name[..i].trim().to_string();
    }
    (name, brackets)
}


/// One page of the Games forum thread list (XenForo): thread id, title with prefixes.
/// Pinned threads (rules, announcements) are counted as service threads and left out.
pub(super) fn forum_page(html: &str) -> Result<crate::external::IndexPage, ProviderError> {
    check_response(200, html)?;
    let mut out = super::IndexPage::default();
    // page numbers of the pager: the largest one is the last page
    let marker = format!("{GAMES_FORUM}page-");
    let mut from = 0;
    while let Some(i) = html[from..].find(&marker) {
        let s = from + i + marker.len();
        let n: String = html[s..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = n.parse::<u32>() {
            out.last_page = Some(out.last_page.map_or(n, |m| m.max(n)));
        }
        from = s;
    }
    let sticky_end = html
        .find("structItemContainer-group--sticky")
        .and_then(|i| {
            html[i + 10..]
                .find("structItemContainer-group")
                .map(|j| i + 10 + j)
        });
    const ITEM: &str = "js-threadListItem-";
    let starts: Vec<usize> = html.match_indices(ITEM).map(|(i, _)| i).collect();
    if starts.is_empty() && !html.contains("structItemContainer") {
        return Err(ProviderError::Parse("нет списка тем".into()));
    }
    for (k, &i) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(html.len());
        let item = &html[i..end];
        let id: String = item[ITEM.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if id.is_empty() {
            continue;
        }
        let sticky = sticky_end.is_some_and(|e| i < e) || item.contains("structItem--sticky");
        if sticky {
            out.service += 1;
            continue;
        }
        // the thread link inside the title block
        let title_html = item
            .find("structItem-title")
            .map(|t| &item[t..])
            .and_then(|t| t.find("</div>").map(|e| &t[..e]))
            .unwrap_or(item);
        let link = title_html
            .match_indices("href=\"/threads/")
            .last()
            .map(|(j, _)| j)
            .and_then(|j| {
                let a = &title_html[j..];
                let g = a.find('>')?;
                let e = a[g..].find("</a>")?;
                Some(html::text(&a[g + 1..g + e]))
            })
            .unwrap_or_default();
        let (name, brackets) = split_title(&link);
        let mut status = "ongoing".to_string();
        let mut engine = String::new();
        for l in html::labels(title_html) {
            if let Some(s) = status_of(&l) {
                status = s.into();
            } else if let Some(e) = engine_of(&l) {
                engine = e.into();
            }
        }
        out.games.push(ExtGame {
            source: ID.into(),
            ext_id: id.clone(),
            title: name,
            developer: if brackets.len() >= 2 {
                brackets.last().cloned().unwrap_or_default()
            } else {
                String::new()
            },
            version: brackets
                .first()
                .cloned()
                .filter(|_| brackets.len() >= 2)
                .unwrap_or_default(),
            status,
            engine,
            page_url: F95Zone::thread_url(&id),
            ..Default::default()
        });
    }
    Ok(out)
}

/// The public game page: title, prefixes, tags, first post (description, developer links).
pub(super) fn game_page(html: &str, id: &str) -> Result<ExtGame, ProviderError> {
    check_response(200, html)?;
    if html.contains("data-template=\"error\"") {
        return Err(ProviderError::Unavailable("тема удалена или скрыта".into()));
    }
    let h1 = html::between(html, "<h1 class=\"p-title-value\"", "</h1>")
        .ok_or_else(|| ProviderError::Parse("нет заголовка темы".into()))?;
    let h1 = h1.split_once('>').map(|x| x.1).unwrap_or(h1);
    // prefix labels: <span class="label ...">Completed</span> (also inside <a class="labelLink">)
    let mut labels = Vec::new();
    let mut rest = h1;
    while let Some(i) = rest.find("<span class=\"label") {
        let Some(gt) = rest[i..].find('>') else { break };
        let s = i + gt + 1;
        let Some(e) = rest[s..].find("</span>") else {
            break;
        };
        labels.push(html::text(&rest[s..s + e]));
        rest = &rest[s + e..];
    }
    let mut title_text = html::text(h1);
    for l in &labels {
        if let Some(t) = title_text.strip_prefix(l.as_str()) {
            title_text = t.trim_start().to_string();
        }
    }
    let title_text = title_text.replace('\n', " ");
    // "Name [v0.5] [Developer]"
    let (name, brackets) = split_title(&title_text);
    let mut status = "ongoing".to_string();
    let mut engine = String::new();
    for l in &labels {
        if let Some(s) = status_of(l) {
            status = s.into();
        } else if let Some(e) = engine_of(l) {
            engine = e.into();
        }
    }
    // tags
    let mut tags = Vec::new();
    if let Some(i) = html.find("js-tagList") {
        let end = html[i..]
            .find("</dd>")
            .or_else(|| html[i..].find("</div>"))
            .unwrap_or(0);
        let list = &html[i..i + end];
        for (pos, href) in html::attr_values(list, "href") {
            let Some(slug) = href.strip_prefix("/tags/") else {
                continue;
            };
            // the name is the link's text ("asset-addon", "3d game"), as in the feed; the
            // address ("3d-game") is only a fallback
            let name = list[pos..]
                .find('>')
                .and_then(|g| list[pos + g + 1..].find("</a>").map(|e| (pos + g + 1, e)))
                .map(|(st, e)| html::text(&list[st..st + e]).to_lowercase())
                .filter(|t| !t.is_empty() && t.len() < 60)
                .unwrap_or_else(|| slug.trim_matches('/').replace('-', " "));
            if !name.is_empty() && !tags.contains(&name) {
                tags.push(name);
            }
        }
    }
    // first post
    // the thread's first post (a notice above it may have a text block of its own)
    let from = html.find("message-threadStarterPost").unwrap_or(0);
    let post_start = html[from..]
        .find("<div class=\"bbWrapper\">")
        .map(|i| from + i)
        .ok_or_else(|| ProviderError::Parse("нет первого сообщения".into()))?;
    let post_end = html[post_start..]
        .find("</article>")
        .map(|e| post_start + e)
        .unwrap_or(html.len());
    let post = &html[post_start..post_end];
    let cut = download_section(&post.to_ascii_lowercase());
    let head = &post[..cut];
    let text = html::text(head);
    let mut links: Vec<OfficialLink> = Vec::new();
    let mut seen = HashSet::new();
    for (pos, href) in html::attr_values(head, "href") {
        let url = html::unescape(href);
        let Some(site) = official_site(&url) else {
            continue;
        };
        if !seen.insert(super::matching::norm_url(&url)) {
            continue;
        }
        let label = head[pos..]
            .find('>')
            .and_then(|g| head[pos + g + 1..].find("</a>").map(|e| (pos + g + 1, e)))
            .map(|(s, e)| html::text(&head[s..s + e]))
            .filter(|l| !l.is_empty() && l.len() < 60)
            .unwrap_or_else(|| site.to_string());
        links.push(OfficialLink {
            site: site.into(),
            label,
            url,
        });
    }
    // the title's "[Developer]" is the site's own short form; the post's field is the fallback
    let developer = brackets
        .last()
        .filter(|_| brackets.len() >= 2)
        .cloned()
        .or_else(|| field(&text, DEVELOPER_FIELDS).map(|d| clean_developer(&d)))
        .filter(|d| !d.is_empty())
        .or_else(|| brackets.last().cloned())
        .unwrap_or_default();
    let version = field(&text, &["Version", "Game Version", "Mod Version"])
        .or_else(|| brackets.first().cloned().filter(|b| b != &developer))
        .unwrap_or_default();
    let updated = field(&text, &["Thread Updated", "Updated", "Release Date"]).and_then(|d| {
        let d = d.trim().to_string();
        let d = d.replace('/', "-");
        let ok = d.len() >= 10
            && d.get(..10).is_some_and(|h| {
                h.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                })
            });
        // 10 ASCII characters were checked above, so this cut is safe
        ok.then(|| d[..10].to_string())
    });
    let cover = html::attr_values(head, "data-src")
        .into_iter()
        .chain(html::attr_values(head, "src"))
        .map(|(_, u)| html::unescape(u))
        .find(|u| {
            u.starts_with("https://attachments.f95zone.to/")
                || u.starts_with("https://preview.f95zone.to/")
        });
    Ok(ExtGame {
        source: ID.into(),
        ext_id: id.to_string(),
        title: name,
        developer,
        version,
        status,
        engine,
        tags,
        cover,
        rating: None,
        likes: 0,
        views: 0,
        updated,
        page_url: F95Zone::thread_url(id),
        description: overview(&text),
        links,
    })
}
