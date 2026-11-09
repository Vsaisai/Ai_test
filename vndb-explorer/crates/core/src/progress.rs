//! Progress reporting for long jobs (import, sync): text log lines + structured progress.

use serde::Serialize;

/// Where a job is. `stage` is a stable id (see [`stage_label`]), `step`/`steps` its position.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Progress {
    pub stage: &'static str,
    pub step: u32,
    pub steps: u32,
    /// Units done in this stage (bytes for download/read, items for sync).
    pub done: u64,
    /// Units in this stage, if known (None = indeterminate).
    pub total: Option<u64>,
    /// "bytes" or "items"
    pub unit: &'static str,
}

pub struct Reporter<'a> {
    log: &'a dyn Fn(&str),
    progress: &'a dyn Fn(&Progress),
}

impl<'a> Reporter<'a> {
    pub fn new(log: &'a dyn Fn(&str), progress: &'a dyn Fn(&Progress)) -> Self {
        Self { log, progress }
    }

    /// Text only (CLI): structured progress is ignored.
    pub fn text(log: &'a dyn Fn(&str)) -> Self {
        Self {
            log,
            progress: &|_| {},
        }
    }

    pub fn log(&self, msg: &str) {
        (self.log)(msg)
    }

    pub fn progress(&self, p: Progress) {
        (self.progress)(&p)
    }

    /// Enter a stage with unknown size.
    pub fn stage(&self, stage: &'static str, step: u32, steps: u32) {
        self.progress(Progress {
            stage,
            step,
            steps,
            done: 0,
            total: None,
            unit: "items",
        });
    }
}

/// Russian label for the UI and notifications.
pub fn stage_label(stage: &str) -> &'static str {
    match stage {
        "download" => "Скачивание базы VNDB",
        "read" => "Чтение архива",
        "tags" => "Подсчёт голосов за теги",
        "links" => "Ссылки и обложки",
        "indexes" => "Построение индексов",
        "derived" => "Языки, платформы и даты",
        "compact" => "Сжатие базы",
        "load" => "Открытие базы",
        "sync_vn" => "Новые новеллы",
        "sync_rel" => "Новые релизы",
        "sync_save" => "Сохранение",
        "sources" => "Индексы каталогов",
        "external" => "Лента F95Zone, страницы",
        "external-audit" => "Список тем форума, страницы",
        "external-check" => "Проверка F95Zone",
        "matching" => "Совпадения с VNDB",
        _ => "Обработка",
    }
}

/// Share of the whole job taken by each stage (for an overall percentage).
pub fn stage_weight(stage: &str) -> f64 {
    match stage {
        "download" => 70.0,
        "read" => 15.0,
        "tags" => 5.0,
        "links" | "indexes" => 2.0,
        "derived" | "compact" => 3.0,
        "load" => 1.0,
        "sync_vn" => 30.0,
        "sync_rel" => 60.0,
        "sync_save" => 9.0,
        "external" | "external-audit" => 95.0,
        "matching" => 5.0,
        _ => 1.0,
    }
}

/// Stage order per job kind (to compute the overall percentage).
pub fn stages_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "import" => &[
            "download", "read", "tags", "links", "indexes", "derived", "compact", "load",
        ],
        "sync" => &["sync_vn", "sync_rel", "sync_save", "load"],
        "external" => &["external", "matching"],
        "external-audit" => &["external-audit", "matching"],
        "external-check" => &["external-check"],
        _ => &["sources"],
    }
}
