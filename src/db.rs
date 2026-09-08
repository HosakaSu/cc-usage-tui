//! Read-only access to cc-switch's SQLite database.
//!
//! cc-switch stores usage in two complementary tables:
//!   * `proxy_request_logs`   - recent, per-request rows (live, written in realtime)
//!   * `usage_daily_rollups`  - older, pre-aggregated daily rows (archived)
//!
//! Their date ranges do NOT overlap (raw rows are pruned once rolled up), so
//! summing both gives full history without double counting.
//!
//! Token accounting differs per source. `input_token_semantics`:
//!   1 => input_tokens ALREADY INCLUDES cache_read (cache-inclusive)
//!   2 => input_tokens EXCLUDES cache_read (fresh input only)
//!   0 => unknown; decide by agent. codex/grokbuild report cache-inclusive,
//!        opencode/pi/claude report fresh-only.
//! Everything is normalized into: fresh input, cache read, cache creation, output.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Which slice of history to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    All,       // rollups (all) + raw logs (all)
    Live,      // raw logs only (the live, recently-written data)
    Days(u32), // raw logs from the last N days (UTC)
    Today,     // raw logs for today (UTC)
}

impl Range {
    pub const ORDER: [Range; 5] = [Range::All, Range::Live, Range::Days(30), Range::Days(7), Range::Today];
    pub fn label(self) -> String {
        match self {
            Range::All => "All history".into(),
            Range::Live => "Live logs".into(),
            Range::Days(n) => format!("Last {n}d"),
            Range::Today => "Today".into(),
        }
    }
    fn includes_rollups(self) -> bool {
        matches!(self, Range::All)
    }
    fn raw_where(self) -> String {
        match self {
            Range::All | Range::Live => String::new(),
            Range::Days(n) => format!(
                "WHERE created_at >= CAST(strftime('%s','now') AS INTEGER) - {}",
                (n as u64) * 86400
            ),
            Range::Today => "WHERE date(created_at,'unixepoch') = date('now')".into(),
        }
    }
}

/// One aggregated row: per-agent (model = None) or per-agent+model.
#[derive(Debug, Clone, Default)]
pub struct Stat {
    pub agent: String,
    pub model: Option<String>,
    pub reqs: i64,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
}

impl Stat {
    pub fn total_input(&self) -> i64 {
        self.fresh + self.cached + self.ccreate
    }
    pub fn total_tokens(&self) -> i64 {
        self.total_input() + self.output
    }
    /// Fraction of input tokens served from cache.
    pub fn cache_hit(&self) -> f64 {
        let ti = self.total_input();
        if ti > 0 {
            self.cached as f64 / ti as f64
        } else {
            0.0
        }
    }
    pub fn label(&self) -> String {
        match &self.model {
            Some(m) => format!("{} · {}", self.agent, m),
            None => self.agent.clone(),
        }
    }
    fn add(&mut self, o: &Stat) {
        self.reqs += o.reqs;
        self.fresh += o.fresh;
        self.cached += o.cached;
        self.ccreate += o.ccreate;
        self.output += o.output;
        self.cost += o.cost;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub db_path: PathBuf,
    pub db_bytes: u64,
    pub db_mtime: Option<SystemTime>,
    pub journal_mode: String,
    pub raw_rows: i64,
    pub rollup_rows: i64,
    pub last_write: Option<SystemTime>,
}

impl Meta {
    pub fn last_write_elapsed(&self) -> Option<Duration> {
        self.last_write
            .map(|t| SystemTime::now().duration_since(t).unwrap_or_default())
    }
}

/// True when the source reports `input_tokens` as already including cache.
fn is_cache_inclusive(agent: &str, sem: i64) -> bool {
    match sem {
        1 => true,
        2 => false,
        _ => matches!(agent, "codex" | "grokbuild"),
    }
}

/// fresh input tokens given the source convention.
fn fresh_input(agent: &str, sem: i64, in_tok: i64, cached: i64, ccreate: i64) -> i64 {
    if is_cache_inclusive(agent, sem) {
        (in_tok - cached - ccreate).max(0)
    } else {
        in_tok.max(0)
    }
}

pub struct Reader {
    conn: Connection,
    path: PathBuf,
}

impl Reader {
    /// Open strictly read-only: never creates -wal/-journal files, never mutates.
    /// Retries briefly in case cc-switch holds the write lock at this instant.
    pub fn open(path: &Path) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut attempt = 0;
        let conn = loop {
            match Connection::open_with_flags(path, flags) {
                Ok(c) => break c,
                Err(e) if attempt < 5 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(150));
                    let _ = &e;
                    continue;
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(e))
                        .with_context(|| format!("cannot open read-only: {}", path.display()))
                }
            }
        };
        // Per-connection: wait up to 2s if cc-switch holds the write lock.
        let _ = conn.busy_timeout(Duration::from_millis(2000));
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn meta(&self) -> Result<Meta> {
        let mut m = Meta {
            db_path: self.path.clone(),
            ..Default::default()
        };
        if let Ok(fs) = std::fs::metadata(&self.path) {
            m.db_bytes = fs.len();
            m.db_mtime = fs.modified().ok();
        }
        m.journal_mode = self
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap_or_else(|_| "?".into());
        m.raw_rows = self
            .conn
            .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
            .unwrap_or(0);
        m.rollup_rows = self
            .conn
            .query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |r| r.get(0))
            .unwrap_or(0);
        let last: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(created_at),0) FROM proxy_request_logs",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if last > 0 {
            m.last_write = Some(UNIX_EPOCH + Duration::from_secs(last as u64));
        }
        Ok(m)
    }

    /// Aggregate stats. `by_model=false` -> per agent; `true` -> per agent+model.
    pub fn stats(&self, range: Range, by_model: bool) -> Result<Vec<Stat>> {
        let mut rows: Vec<Stat> = Vec::new();

        // raw logs (always included)
        let where_sql = range.raw_where();
        let (cols, group) = if by_model {
            ("app_type, model,", "app_type, model")
        } else {
            ("app_type,", "app_type")
        };
        let raw_sql = format!(
            "SELECT {cols} COALESCE(input_token_semantics,0), COUNT(*), \
                    COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0), \
                    COALESCE(SUM(cache_creation_tokens),0), COALESCE(SUM(output_tokens),0), \
                    COALESCE(SUM(CAST(total_cost_usd AS REAL)),0) \
             FROM proxy_request_logs {where_sql} \
             GROUP BY {group}, COALESCE(input_token_semantics,0)"
        );
        self.collect(&raw_sql, by_model, &mut rows)?;

        // rollups (only for the All range)
        if range.includes_rollups() {
            let roll_sql = format!(
                "SELECT {cols} COALESCE(input_token_semantics,0), COALESCE(SUM(request_count),0), \
                        COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0), \
                        COALESCE(SUM(cache_creation_tokens),0), COALESCE(SUM(output_tokens),0), \
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)),0) \
                 FROM usage_daily_rollups \
                 GROUP BY {group}, COALESCE(input_token_semantics,0)"
            );
            self.collect(&roll_sql, by_model, &mut rows)?;
        }

        Ok(merge(rows, by_model))
    }

    fn collect(&self, sql: &str, by_model: bool, out: &mut Vec<Stat>) -> Result<()> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut q = stmt.query([])?;
        while let Some(r) = q.next()? {
            // index of sem shifts by 1 when the model column is present
            let (i_sem, i_reqs) = if by_model { (2, 3) } else { (1, 2) };
            let agent: String = r.get(0)?;
            let model: Option<String> = if by_model { Some(r.get(1)?) } else { None };
            let sem: i64 = r.get(i_sem)?;
            let reqs: i64 = r.get(i_reqs)?;
            let in_tok: i64 = r.get(i_reqs + 1)?;
            let cached: i64 = r.get(i_reqs + 2)?;
            let ccreate: i64 = r.get(i_reqs + 3)?;
            let output: i64 = r.get(i_reqs + 4)?;
            let cost: f64 = r.get(i_reqs + 5)?;
            let fresh = fresh_input(&agent, sem, in_tok, cached, ccreate);
            out.push(Stat {
                agent,
                model,
                reqs,
                fresh,
                cached,
                ccreate,
                output,
                cost,
            });
        }
        Ok(())
    }
}

/// Sum rows that share the same key (agent, or agent+model) across sem groups
/// and across the two tables.
fn merge(rows: Vec<Stat>, by_model: bool) -> Vec<Stat> {
    let mut map: BTreeMap<String, Stat> = BTreeMap::new();
    for r in rows {
        let key = if by_model { r.label() } else { r.agent.clone() };
        let e = map.entry(key).or_insert_with(|| Stat {
            agent: r.agent.clone(),
            model: r.model.clone(),
            ..Default::default()
        });
        e.add(&r);
    }
    map.into_values().collect()
}

pub fn total_of(stats: &[Stat]) -> Stat {
    let mut t = Stat {
        agent: "TOTAL".into(),
        model: None,
        ..Default::default()
    };
    for s in stats {
        t.add(s);
    }
    t
}
