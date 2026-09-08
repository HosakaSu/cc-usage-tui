use crate::db::{total_of, Meta, Range, Reader, Stat};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Agents,
    Models,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Agents => "Agents",
            View::Models => "Models",
        }
    }
    pub fn toggle(self) -> Self {
        match self {
            View::Agents => View::Models,
            View::Models => View::Agents,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cost,
    TotalTokens,
    Reqs,
    CacheHit,
    Name,
}

impl SortKey {
    pub const ORDER: [SortKey; 5] = [SortKey::Cost, SortKey::TotalTokens, SortKey::Reqs, SortKey::CacheHit, SortKey::Name];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cost => "Cost",
            SortKey::TotalTokens => "Total tokens",
            SortKey::Reqs => "Requests",
            SortKey::CacheHit => "Cache hit",
            SortKey::Name => "Name",
        }
    }
}

pub struct App {
    pub db_path: PathBuf,
    pub range: Range,
    pub view: View,
    pub sort: SortKey,
    pub stats: Vec<Stat>,
    pub total: Stat,
    pub meta: Meta,
    pub status: String,
    pub status_error: bool,
    pub selected: usize,
    pub quit: bool,
    pub last_refresh: Option<SystemTime>,
    pub last_mtime: Option<SystemTime>,
    pub refresh_count: u64,
}

impl App {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            range: Range::All,
            view: View::Agents,
            sort: SortKey::Cost,
            stats: Vec::new(),
            total: total_of(&[]),
            meta: Meta::default(),
            status: "loading…".into(),
            status_error: false,
            selected: 0,
            quit: false,
            last_refresh: None,
            last_mtime: None,
            refresh_count: 0,
        }
    }

    /// Re-read the DB. On lock/error, keep the last good data and surface a
    /// status message (never crash the UI).
    pub fn refresh(&mut self, reader: &Reader) {
        match self.query(reader) {
            Ok(()) => {
                self.refresh_count += 1;
                self.last_refresh = Some(SystemTime::now());
            }
            Err(e) => {
                let msg = e.to_string();
                self.status = if msg.contains("locked") || msg.contains("busy") {
                    "db busy (cc-switch writing) — retrying…".into()
                } else {
                    format!("error: {msg}")
                };
                self.status_error = true;
                // still try to update mtime so we re-trigger on next change
                if let Ok(fs) = std::fs::metadata(&self.db_path) {
                    self.last_mtime = fs.modified().ok();
                }
            }
        }
    }

    fn query(&mut self, reader: &Reader) -> anyhow::Result<()> {
        let meta = reader.meta()?;
        let by_model = matches!(self.view, View::Models);
        let mut stats = reader.stats(self.range, by_model)?;
        sort_stats(&mut stats, self.sort);
        let total = total_of(&stats);
        if self.selected >= stats.len() {
            self.selected = stats.len().saturating_sub(1);
        }
        self.meta = meta;
        self.stats = stats;
        self.total = total;
        self.status = "ok".into();
        self.status_error = false;
        Ok(())
    }

    pub fn cycle_range(&mut self) {
        let i = Range::ORDER.iter().position(|r| *r == self.range).unwrap_or(0);
        self.range = Range::ORDER[(i + 1) % Range::ORDER.len()];
    }

    pub fn cycle_sort(&mut self) {
        let i = SortKey::ORDER.iter().position(|s| *s == self.sort).unwrap_or(0);
        self.sort = SortKey::ORDER[(i + 1) % SortKey::ORDER.len()];
        sort_stats(&mut self.stats, self.sort);
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.stats.is_empty() && self.selected + 1 < self.stats.len() {
            self.selected += 1;
        }
    }
}

pub fn sort_stats(stats: &mut [Stat], key: SortKey) {
    match key {
        SortKey::Cost => stats.sort_by(|a, b| b.cost.total_cmp(&a.cost)),
        SortKey::TotalTokens => stats.sort_by_key(|s| std::cmp::Reverse(s.total_tokens())),
        SortKey::Reqs => stats.sort_by_key(|s| std::cmp::Reverse(s.reqs)),
        SortKey::CacheHit => stats.sort_by(|a, b| b.cache_hit().total_cmp(&a.cache_hit())),
        SortKey::Name => stats.sort_by(|a, b| a.label().cmp(&b.label())),
    }
}
