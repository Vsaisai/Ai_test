//! Language handling: which languages count as "readable" and which title to show.

use crate::api::Vn;
use serde_json::{json, Value};

/// Languages written mainly with CJK characters (hieroglyphs / hangul).
pub const CJK: &[&str] = &["ja", "zh", "zh-Hans", "zh-Hant", "ko"];

/// Every language code known to VNDB (from /kana/schema).
pub const ALL: &[&str] = &[
    "ar", "eu", "be", "bg", "bs", "ca", "ck", "zh", "zh-Hans", "zh-Hant", "hr", "cs", "da", "nl",
    "en", "eo", "et", "fi", "fr", "gl", "de", "el", "he", "hi", "hu", "ga", "id", "it", "iu", "ja",
    "kk", "ko", "la", "lv", "lt", "mk", "ms", "ne", "no", "fa", "pl", "pt-br", "pt-pt", "ro", "ru",
    "gd", "sr", "sk", "sl", "es", "sv", "ta", "th", "tr", "uk", "ur", "vi",
];

pub fn is_cjk(lang: &str) -> bool {
    CJK.contains(&lang)
}

/// How results are restricted by language.
#[derive(Debug, Clone)]
pub struct LangPref {
    /// Preferred languages in order (first = most wanted). Used for titles & sorting links.
    pub order: Vec<String>,
    /// Languages a VN must be available in (any of). `None` = no restriction.
    pub accept: Option<Vec<String>>,
}

impl LangPref {
    /// `langs` = explicit user list, `any` = disable filtering.
    /// Default: English preferred, any non-CJK language accepted.
    pub fn new(langs: Option<&str>, any: bool) -> Self {
        let explicit: Vec<String> = langs
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let order = if explicit.is_empty() {
            vec!["en".to_string()]
        } else {
            explicit.clone()
        };
        let accept = if any {
            None
        } else if !explicit.is_empty() {
            Some(explicit)
        } else {
            Some(
                ALL.iter()
                    .filter(|l| !is_cjk(l))
                    .map(|l| l.to_string())
                    .collect(),
            )
        };
        Self { order, accept }
    }

    /// VNDB filter expression, if any.
    pub fn filter(&self) -> Option<Value> {
        let acc = self.accept.as_ref()?;
        let mut f = vec![json!("or")];
        f.extend(acc.iter().map(|l| json!(["lang", "=", l])));
        Some(Value::Array(f))
    }

    pub fn accepts(&self, lang: &str) -> bool {
        match &self.accept {
            None => true,
            Some(a) => a.iter().any(|l| l == lang),
        }
    }

    /// Lower = better. Preferred languages first, then other accepted non-CJK, then the rest.
    pub fn rank(&self, lang: &str) -> usize {
        if let Some(i) = self.order.iter().position(|l| l == lang) {
            i
        } else if self.accepts(lang) && !is_cjk(lang) {
            100
        } else {
            1000
        }
    }

    /// Languages of a VN, best first.
    pub fn sorted_langs(&self, langs: &[String]) -> Vec<String> {
        let mut v = langs.to_vec();
        v.sort_by_key(|l| (self.rank(l), l.clone()));
        v
    }
}

pub fn has_cjk_chars(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32,
            0x3040..=0x30FF   // hiragana, katakana
            | 0x3400..=0x4DBF // CJK ext A
            | 0x4E00..=0x9FFF // CJK unified
            | 0xAC00..=0xD7AF // hangul
            | 0xF900..=0xFAFF
            | 0xFF66..=0xFF9F)
    })
}

/// Best readable title: preferred-language title, else any non-CJK title, else romanization.
pub fn display_title(vn: &Vn, pref: &LangPref) -> String {
    let mut titles: Vec<_> = vn.titles.iter().collect();
    titles.sort_by_key(|t| (pref.rank(&t.lang), !t.official, !t.main));
    for t in &titles {
        if is_cjk(&t.lang) {
            continue;
        }
        if !has_cjk_chars(&t.title) {
            return t.title.clone();
        }
        if let Some(l) = &t.latin {
            return l.clone();
        }
    }
    if let Some(l) = vn
        .titles
        .iter()
        .find(|t| t.main)
        .and_then(|t| t.latin.clone())
    {
        return l;
    }
    vn.title.clone()
}

/// The original-script title when it differs from the shown one.
pub fn original_title(vn: &Vn, shown: &str) -> Option<String> {
    vn.titles
        .iter()
        .find(|t| t.main)
        .map(|t| t.title.clone())
        .or_else(|| vn.alttitle.clone())
        .filter(|t| t != shown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Title;

    fn vn(titles: Vec<Title>) -> Vn {
        serde_json::from_value::<Vn>(json!({"id":"v1","title":"Romaji Title"}))
            .map(|mut v| {
                v.titles = titles;
                v
            })
            .unwrap()
    }
    fn t(lang: &str, title: &str, latin: Option<&str>, main: bool) -> Title {
        Title {
            lang: lang.into(),
            title: title.into(),
            latin: latin.map(Into::into),
            official: true,
            main,
        }
    }

    #[test]
    fn prefers_english_then_other_readable() {
        let p = LangPref::new(None, false);
        let v = vn(vec![
            t("ja", "日本語", Some("Nihongo"), true),
            t("ru", "Русский", None, false),
            t("en", "English", None, false),
        ]);
        assert_eq!(display_title(&v, &p), "English");
        let v = vn(vec![
            t("ja", "日本語", Some("Nihongo"), true),
            t("ru", "Русский", None, false),
        ]);
        assert_eq!(display_title(&v, &p), "Русский");
        let v = vn(vec![t("ja", "日本語", Some("Nihongo"), true)]);
        assert_eq!(display_title(&v, &p), "Nihongo");
    }

    #[test]
    fn default_filter_excludes_cjk() {
        let p = LangPref::new(None, false);
        assert!(p.accepts("en") && p.accepts("ru"));
        assert!(!p.accepts("ja") && !p.accepts("zh-Hans"));
        assert!(LangPref::new(None, true).filter().is_none());
        let p = LangPref::new(Some("ru,en"), false);
        assert_eq!(p.order, vec!["ru", "en"]);
        assert!(!p.accepts("de"));
    }
}
