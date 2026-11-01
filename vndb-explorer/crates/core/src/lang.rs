//! Languages: which ones count as "readable" (non-CJK) and how titles are picked.

pub const CJK: &[&str] = &["ja", "zh", "zh-Hans", "zh-Hant", "ko"];

pub fn is_cjk(lang: &str) -> bool {
    CJK.contains(&lang)
}

pub fn has_cjk_chars(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F)
    })
}

/// Availability of a language for a VN: 0 = original/official, 1 = fan translation, 2 = machine translation only.
pub type LangKind = u8;

/// Rank of a language for display: preferred list first, then other non-CJK, then CJK.
pub fn rank(order: &[String], lang: &str) -> usize {
    if let Some(i) = order.iter().position(|l| l == lang) {
        i
    } else if !is_cjk(lang) {
        100
    } else {
        1000
    }
}
