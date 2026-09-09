//! Read-only access to cc-switch's SQLite database.
//!
//! Two complementary, non-overlapping tables (raw rows are pruned once rolled up):
//!   * `proxy_request_logs`   - recent per-request rows (live, written in realtime)
//!   * `usage_daily_rollups`  - older pre-aggregated daily rows (archived)
//!
//! All aggregation SQL mirrors cc-switch's own Dashboard conventions so the TUI
//! numbers match the cc-switch web UI:
//!   * fresh-input normalization via `input_token_semantics` (+ agent rules)
//!   * effective model  = COALESCE(NULLIF(pricing_model,''), model)
//!   * display agent    = claude-desktop -> claude
//!   * provider name    = providers.name, else a "(Session)" fallback
//!   * cross-source dedup (proxy vs session) — a no-op when only one source exists
//!   * time ranges use LOCAL dates (chrono) and merge raw detail + full-day rollups

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OpenFlags, Row, ToSql};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Range / TimeBounds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Range {
    #[default]
    All,
    Live, // raw logs only
    Days(u32),
    Today,
}

impl Range {
    pub const ORDER: [Range; 5] = [Range::All, Range::Live, Range::Days(30), Range::Days(7), Range::Today];
    pub fn label(self) -> String {
        match self {
            Range::All => "All".into(),
            Range::Live => "Live".into(),
            Range::Days(n) => format!("{n}d"),
            Range::Today => "Today".into(),
        }
    }
    /// Hourly trend buckets for sub-day ranges, daily otherwise.
    pub fn hourly(self) -> bool {
        matches!(self, Range::Today)
    }

    /// Resolve to concrete bounds using LOCAL time (matches cc-switch Dashboard).
    pub fn bounds(self) -> TimeBounds {
        let now = Local::now();
        match self {
            Range::All => TimeBounds {
                include_raw: true,
                include_rollups: true,
                ..Default::default()
            },
            Range::Live => TimeBounds {
                include_raw: true,
                include_rollups: false,
                ..Default::default()
            },
            Range::Today => {
                let d = now.date_naive();
                TimeBounds {
                    start_ts: Some(local_midnight_ts(d)),
                    end_ts: Some(now.timestamp()),
                    rollup_start: Some(d.to_string()),
                    rollup_end: Some(d.to_string()),
                    include_raw: true,
                    include_rollups: true,
                }
            }
            Range::Days(n) => {
                let end_d = now.date_naive();
                let start_d = end_d
                    .checked_sub_days(chrono::Days::new((n.max(1) - 1) as u64))
                    .unwrap_or(end_d);
                TimeBounds {
                    start_ts: Some(local_midnight_ts(start_d)),
                    end_ts: Some(now.timestamp()),
                    rollup_start: Some(start_d.to_string()),
                    rollup_end: Some(end_d.to_string()),
                    include_raw: true,
                    include_rollups: true,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TimeBounds {
    pub start_ts: Option<i64>,
    pub end_ts: Option<i64>,
    pub rollup_start: Option<String>, // YYYY-MM-DD
    pub rollup_end: Option<String>,
    pub include_raw: bool,
    pub include_rollups: bool,
}

fn local_midnight_ts(d: NaiveDate) -> i64 {
    let nd = d.and_hms_opt(0, 0, 0).unwrap();
    if let Some(dt) = Local.from_local_datetime(&nd).earliest() {
        dt.timestamp()
    } else {
        Local.from_utc_datetime(&nd).timestamp()
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct UsageFilter {
    pub range: Range,
    pub agent: Option<String>,
    pub provider: Option<ProviderKey>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderKey {
    pub app_type: String,
    pub provider_id: String,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Result structs
// ---------------------------------------------------------------------------

macro_rules! token_agg {
    ($t:ty) => {
        impl $t {
            pub fn total_input(&self) -> i64 {
                self.fresh + self.cached + self.ccreate
            }
            pub fn total_tokens(&self) -> i64 {
                self.total_input() + self.output
            }
            pub fn cache_hit(&self) -> f64 {
                let ti = self.total_input();
                if ti > 0 {
                    self.cached as f64 / ti as f64
                } else {
                    0.0
                }
            }
        }
    };
}

#[derive(Debug, Clone, Default)]
pub struct UsageSummary {
    pub reqs: i64,
    pub success: i64,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
}
token_agg!(UsageSummary);
impl UsageSummary {
    pub fn success_rate(&self) -> f64 {
        if self.reqs > 0 {
            self.success as f64 / self.reqs as f64
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentStat {
    pub agent: String,
    pub reqs: i64,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
}
token_agg!(AgentStat);

#[derive(Debug, Clone, Default)]
pub struct ModelStat {
    pub model: String,
    pub reqs: i64,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
}
token_agg!(ModelStat);
impl ModelStat {
    pub fn avg_tokens_per_req(&self) -> f64 {
        if self.reqs > 0 {
            self.total_tokens() as f64 / self.reqs as f64
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderStat {
    pub app_type: String,
    pub provider_id: String,
    pub provider_name: String,
    pub reqs: i64,
    pub success: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub avg_latency_ms: f64,
}
impl ProviderStat {
    pub fn success_rate(&self) -> f64 {
        if self.reqs > 0 {
            self.success as f64 / self.reqs as f64
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrendPoint {
    pub ts: i64,
    pub label: String,
    pub reqs: i64,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
}
token_agg!(TrendPoint);

#[derive(Debug, Clone, Default)]
pub struct RequestLog {
    pub request_id: String,
    pub created_at: i64,
    pub app_type: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub request_model: Option<String>,
    pub pricing_model: Option<String>,
    pub fresh: i64,
    pub cached: i64,
    pub ccreate: i64,
    pub output: i64,
    pub cost: f64,
    pub latency_ms: i64,
    pub first_token_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status_code: i64,
    pub error_message: Option<String>,
    pub data_source: String,
}

#[derive(Debug, Clone, Default)]
pub struct RequestPage {
    pub rows: Vec<RequestLog>,
    pub total: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FilterOptions {
    pub agents: Vec<String>,
    pub providers: Vec<ProviderKey>,
    pub models: Vec<String>,
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
        self.last_write.map(|t| SystemTime::now().duration_since(t).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// SQL helpers (mirror cc-switch Dashboard conventions)
// ---------------------------------------------------------------------------

fn fresh_input_sql(a: &str) -> String {
    format!(
        "CASE \
           WHEN {a}.input_token_semantics = 2 THEN {a}.input_tokens \
           WHEN {a}.app_type IN ('codex','gemini','grokbuild') AND {a}.input_token_semantics = 1 \
                AND {a}.input_tokens >= {a}.cache_read_tokens + {a}.cache_creation_tokens \
             THEN {a}.input_tokens - {a}.cache_read_tokens - {a}.cache_creation_tokens \
           WHEN {a}.app_type IN ('codex','gemini','grokbuild') AND {a}.input_token_semantics = 0 \
                AND {a}.input_tokens >= {a}.cache_read_tokens \
             THEN {a}.input_tokens - {a}.cache_read_tokens \
           ELSE {a}.input_tokens \
         END"
    )
}

fn display_app_sql(a: &str) -> String {
    format!("CASE WHEN {a}.app_type = 'claude-desktop' THEN 'claude' ELSE {a}.app_type END")
}

fn effective_model_sql(a: &str) -> String {
    format!("COALESCE(NULLIF({a}.pricing_model, ''), {a}.model)")
}

fn provider_name_sql(l: &str, p: &str) -> String {
    format!(
        "COALESCE({p}.name, CASE {l}.provider_id \
           WHEN '_session' THEN 'Claude (Session)' \
           WHEN '_codex_session' THEN 'Codex (Session)' \
           WHEN '_gemini_session' THEN 'Gemini (Session)' \
           WHEN '_opencode_session' THEN 'OpenCode (Session)' \
           WHEN '_grok_session' THEN 'Grok Build (Session)' \
           WHEN '_pi_session' THEN 'Pi (Session)' \
           ELSE {l}.provider_id END)"
    )
}

fn effective_usage_filter(l: &str) -> String {
    format!(
        "{l}.request_id IN ( \
           SELECT pick FROM ( \
             SELECT request_id AS pick, ROW_NUMBER() OVER ( \
               PARTITION BY app_type, input_tokens, output_tokens, cache_read_tokens, \
                            cache_creation_tokens, created_at \
               ORDER BY CASE WHEN COALESCE(data_source,'proxy')='proxy' THEN 0 ELSE 1 END, request_id \
             ) rn FROM proxy_request_logs \
           ) WHERE rn = 1)"
    )
}

fn total_tokens_sql(a: &str) -> String {
    format!(
        "({} + {a}.cache_read_tokens + {a}.cache_creation_tokens + {a}.output_tokens)",
        fresh_input_sql(a)
    )
}

// ---------------------------------------------------------------------------
// WHERE builder (positional params pushed in SQL order)
// ---------------------------------------------------------------------------

struct Ctx<'p> {
    filter: &'p UsageFilter,
    pid: &'p Option<String>,
    papp: &'p Option<String>,
    bounds: &'p TimeBounds,
}

fn build_where<'p>(ctx: &Ctx<'p>, alias: &str, is_raw: bool, params: &mut Vec<&'p dyn ToSql>) -> String {
    let mut w = String::from("1=1");
    w.push_str(&format!(" AND (? IS NULL OR {} = ?)", display_app_sql(alias)));
    params.push(&ctx.filter.agent);
    params.push(&ctx.filter.agent);

    w.push_str(&format!(" AND (? IS NULL OR {} = ?)", effective_model_sql(alias)));
    params.push(&ctx.filter.model);
    params.push(&ctx.filter.model);

    w.push_str(&format!(
        " AND (? IS NULL OR ({alias}.provider_id = ? AND (? IS NULL OR {alias}.app_type = ?)))"
    ));
    params.push(ctx.pid);
    params.push(ctx.pid);
    params.push(ctx.papp);
    params.push(ctx.papp);

    if is_raw {
        w.push_str(&format!(" AND (? IS NULL OR {alias}.created_at >= ?)"));
        params.push(&ctx.bounds.start_ts);
        params.push(&ctx.bounds.start_ts);
        w.push_str(&format!(" AND (? IS NULL OR {alias}.created_at <= ?)"));
        params.push(&ctx.bounds.end_ts);
        params.push(&ctx.bounds.end_ts);
        w.push_str(&format!(" AND {}", effective_usage_filter(alias)));
    } else {
        w.push_str(&format!(" AND (? IS NULL OR {alias}.date >= ?)"));
        params.push(&ctx.bounds.rollup_start);
        params.push(&ctx.bounds.rollup_start);
        w.push_str(&format!(" AND (? IS NULL OR {alias}.date <= ?)"));
        params.push(&ctx.bounds.rollup_end);
        params.push(&ctx.bounds.rollup_end);
    }
    w
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

pub struct Reader {
    conn: Connection,
    path: PathBuf,
}

impl Reader {
    /// Open strictly read-only; retries briefly if cc-switch holds the write lock.
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
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(e))
                        .with_context(|| format!("cannot open read-only: {}", path.display()))
                }
            }
        };
        let _ = conn.busy_timeout(Duration::from_millis(2000));
        Ok(Self { conn, path: path.to_path_buf() })
    }

    /// Wrap an existing connection (used by tests with an in-memory DB).
    pub fn from_connection(conn: Connection) -> Self {
        Self { conn, path: PathBuf::from(":memory:") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn meta(&self) -> Result<Meta> {
        let mut m = Meta { db_path: self.path.clone(), ..Default::default() };
        if self.path != Path::new(":memory:") {
            if let Ok(fs) = std::fs::metadata(&self.path) {
                m.db_bytes = fs.len();
                m.db_mtime = fs.modified().ok();
            }
        }
        m.journal_mode = self
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap_or_else(|_| "?".into());
        m.raw_rows = self.scalar_i64("SELECT COUNT(*) FROM proxy_request_logs").unwrap_or(0);
        m.rollup_rows = self.scalar_i64("SELECT COUNT(*) FROM usage_daily_rollups").unwrap_or(0);
        let last = self
            .scalar_i64("SELECT COALESCE(MAX(created_at),0) FROM proxy_request_logs")
            .unwrap_or(0);
        if last > 0 {
            m.last_write = Some(UNIX_EPOCH + Duration::from_secs(last as u64));
        }
        Ok(m)
    }

    fn scalar_i64(&self, sql: &str) -> Result<i64> {
        Ok(self.conn.query_row(sql, [], |r| r.get(0))?)
    }

    // ---- summary ----
    pub fn summary(&self, filter: &UsageFilter) -> Result<UsageSummary> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!(
                "SELECT 1 reqs, CASE WHEN l.status_code BETWEEN 200 AND 299 THEN 1 ELSE 0 END succ, \
                        {} fresh, l.cache_read_tokens cached, l.cache_creation_tokens ccreate, \
                        l.output_tokens output, CAST(l.total_cost_usd AS REAL) cost \
                 FROM proxy_request_logs l WHERE {}",
                fresh_input_sql("l"),
                w
            ));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!(
                "SELECT r.request_count reqs, r.success_count succ, {} fresh, \
                        r.cache_read_tokens cached, r.cache_creation_tokens ccreate, \
                        r.output_tokens output, CAST(r.total_cost_usd AS REAL) cost \
                 FROM usage_daily_rollups r WHERE {}",
                fresh_input_sql("r"),
                w
            ));
        }
        let sql = format!(
            "SELECT COALESCE(SUM(reqs),0), COALESCE(SUM(succ),0), COALESCE(SUM(fresh),0), \
                    COALESCE(SUM(cached),0), COALESCE(SUM(ccreate),0), COALESCE(SUM(output),0), \
                    COALESCE(SUM(cost),0) FROM ( {} )",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let r = q.next()?.context("summary row")?;
        Ok(UsageSummary {
            reqs: r.get(0)?,
            success: r.get(1)?,
            fresh: r.get(2)?,
            cached: r.get(3)?,
            ccreate: r.get(4)?,
            output: r.get(5)?,
            cost: r.get(6)?,
        })
    }

    // ---- agent / model stats (shared grouped builder) ----
    pub fn agent_stats(&self, filter: &UsageFilter) -> Result<Vec<AgentStat>> {
        let raw_key = display_app_sql("l");
        let roll_key = display_app_sql("r");
        self.grouped_stats(filter, &raw_key, &roll_key, |r| {
            Ok(AgentStat {
                agent: r.get(0)?,
                reqs: r.get(1)?,
                fresh: r.get(2)?,
                cached: r.get(3)?,
                ccreate: r.get(4)?,
                output: r.get(5)?,
                cost: r.get(6)?,
            })
        })
    }

    pub fn model_stats(&self, filter: &UsageFilter) -> Result<Vec<ModelStat>> {
        let raw_key = effective_model_sql("l");
        let roll_key = effective_model_sql("r");
        self.grouped_stats(filter, &raw_key, &roll_key, |r| {
            Ok(ModelStat {
                model: r.get(0)?,
                reqs: r.get(1)?,
                fresh: r.get(2)?,
                cached: r.get(3)?,
                ccreate: r.get(4)?,
                output: r.get(5)?,
                cost: r.get(6)?,
            })
        })
    }

    fn grouped_stats<T, F>(&self, filter: &UsageFilter, raw_key: &str, roll_key: &str, map: F) -> Result<Vec<T>>
    where
        F: Fn(&Row) -> rusqlite::Result<T>,
    {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!(
                "SELECT {raw_key} k, COUNT(*) reqs, SUM({}) fresh, SUM(l.cache_read_tokens) cached, \
                        SUM(l.cache_creation_tokens) ccreate, SUM(l.output_tokens) output, \
                        SUM(CAST(l.total_cost_usd AS REAL)) cost \
                 FROM proxy_request_logs l WHERE {w} GROUP BY 1",
                fresh_input_sql("l")
            ));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!(
                "SELECT {roll_key} k, SUM(r.request_count) reqs, SUM({}) fresh, \
                        SUM(r.cache_read_tokens) cached, SUM(r.cache_creation_tokens) ccreate, \
                        SUM(r.output_tokens) output, SUM(CAST(r.total_cost_usd AS REAL)) cost \
                 FROM usage_daily_rollups r WHERE {w} GROUP BY 1",
                fresh_input_sql("r")
            ));
        }
        let sql = format!(
            "SELECT k, SUM(reqs), SUM(fresh), SUM(cached), SUM(ccreate), SUM(output), SUM(cost) \
             FROM ( {} ) GROUP BY k ORDER BY SUM(cost) DESC",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push(map(r)?);
        }
        Ok(out)
    }

    // ---- provider stats ----
    pub fn provider_stats(&self, filter: &UsageFilter) -> Result<Vec<ProviderStat>> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!(
                "SELECT l.provider_id pid, l.app_type app, {} pname, 1 reqs, \
                        CASE WHEN l.status_code BETWEEN 200 AND 299 THEN 1 ELSE 0 END succ, \
                        {} toks, CAST(l.total_cost_usd AS REAL) cost, l.latency_ms lat \
                 FROM proxy_request_logs l LEFT JOIN providers p ON p.id = l.provider_id AND p.app_type = l.app_type \
                 WHERE {}",
                provider_name_sql("l", "p"),
                total_tokens_sql("l"),
                w
            ));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!(
                "SELECT r.provider_id pid, r.app_type app, {} pname, r.request_count reqs, \
                        r.success_count succ, {} toks, CAST(r.total_cost_usd AS REAL) cost, \
                        r.avg_latency_ms * r.request_count lat \
                 FROM usage_daily_rollups r LEFT JOIN providers p ON p.id = r.provider_id AND p.app_type = r.app_type \
                 WHERE {}",
                provider_name_sql("r", "p"),
                total_tokens_sql("r"),
                w
            ));
        }
        let sql = format!(
            "SELECT pid, app, MAX(pname), SUM(reqs), SUM(succ), SUM(toks), SUM(cost), \
                    CASE WHEN SUM(reqs)>0 THEN CAST(SUM(lat) AS REAL)/SUM(reqs) ELSE 0 END \
             FROM ( {} ) GROUP BY pid, app ORDER BY SUM(cost) DESC",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push(ProviderStat {
                provider_id: r.get(0)?,
                app_type: r.get(1)?,
                provider_name: r.get(2)?,
                reqs: r.get(3)?,
                success: r.get(4)?,
                total_tokens: r.get(5)?,
                cost: r.get(6)?,
                avg_latency_ms: r.get(7)?,
            });
        }
        Ok(out)
    }

    // ---- trend ----
    pub fn trend(&self, filter: &UsageFilter) -> Result<Vec<TrendPoint>> {
        if filter.range.hourly() {
            self.trend_hourly(filter)
        } else {
            self.trend_daily(filter)
        }
    }

    fn trend_hourly(&self, filter: &UsageFilter) -> Result<Vec<TrendPoint>> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut where_params: Vec<&dyn ToSql> = Vec::new();
        let w = build_where(&ctx, "l", true, &mut where_params);
        let base = bounds.start_ts.unwrap_or(0);
        let sql = format!(
            "SELECT CAST((l.created_at - ?) / 3600 AS INTEGER) bucket, COUNT(*), SUM({}), \
                    SUM(l.cache_read_tokens), SUM(l.cache_creation_tokens), SUM(l.output_tokens), \
                    SUM(CAST(l.total_cost_usd AS REAL)) \
             FROM proxy_request_logs l WHERE {} GROUP BY bucket ORDER BY bucket",
            fresh_input_sql("l"),
            w
        );
        let mut params: Vec<&dyn ToSql> = Vec::with_capacity(where_params.len() + 1);
        params.push(&base);
        params.extend(where_params.iter().copied());

        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            let bucket: i64 = r.get(0)?;
            let ts = base + bucket * 3600;
            out.push(TrendPoint {
                ts,
                label: fmt_hour_local(ts),
                reqs: r.get(1)?,
                fresh: r.get(2)?,
                cached: r.get(3)?,
                ccreate: r.get(4)?,
                output: r.get(5)?,
                cost: r.get(6)?,
            });
        }
        let end = bounds.end_ts.unwrap_or_else(|| Local::now().timestamp());
        Ok(fill_hourly_gaps(out, end))
    }

    fn trend_daily(&self, filter: &UsageFilter) -> Result<Vec<TrendPoint>> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!(
                "SELECT date(l.created_at,'unixepoch','localtime') d, 1 reqs, {} fresh, \
                        l.cache_read_tokens cached, l.cache_creation_tokens ccreate, \
                        l.output_tokens output, CAST(l.total_cost_usd AS REAL) cost \
                 FROM proxy_request_logs l WHERE {}",
                fresh_input_sql("l"),
                w
            ));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!(
                "SELECT r.date d, r.request_count reqs, {} fresh, r.cache_read_tokens cached, \
                        r.cache_creation_tokens ccreate, r.output_tokens output, \
                        CAST(r.total_cost_usd AS REAL) cost \
                 FROM usage_daily_rollups r WHERE {}",
                fresh_input_sql("r"),
                w
            ));
        }
        let sql = format!(
            "SELECT d, SUM(reqs), SUM(fresh), SUM(cached), SUM(ccreate), SUM(output), SUM(cost) \
             FROM ( {} ) WHERE d IS NOT NULL GROUP BY d ORDER BY d",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            let d: String = r.get(0)?;
            out.push(TrendPoint {
                ts: date_str_to_local_ts(&d),
                label: short_date(&d),
                reqs: r.get(1)?,
                fresh: r.get(2)?,
                cached: r.get(3)?,
                ccreate: r.get(4)?,
                output: r.get(5)?,
                cost: r.get(6)?,
            });
        }
        Ok(out)
    }

    // ---- request logs (raw only, paginated) ----
    pub fn request_logs(
        &self,
        filter: &UsageFilter,
        status: Option<i64>,
        page: u32,
        page_size: u32,
    ) -> Result<RequestPage> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };

        let mut where_params: Vec<&dyn ToSql> = Vec::new();
        let w = build_where(&ctx, "l", true, &mut where_params);
        let status_sql = " AND (? IS NULL OR l.status_code = ?)";
        let limit = page_size.max(1) as i64;
        let offset = page as i64 * limit;

        let count_sql = format!("SELECT COUNT(*) FROM proxy_request_logs l WHERE {w}{status_sql}");
        let mut count_params: Vec<&dyn ToSql> = where_params.clone();
        count_params.push(&status);
        count_params.push(&status);
        let total: u32 = {
            let mut stmt = self.conn.prepare(&count_sql)?;
            stmt.query_row(count_params.as_slice(), |r| r.get(0)).unwrap_or(0)
        };

        let rows_sql = format!(
            "SELECT l.request_id, l.created_at, l.app_type, l.provider_id, {}, l.model, \
                    l.request_model, l.pricing_model, {}, l.cache_read_tokens, l.cache_creation_tokens, \
                    l.output_tokens, CAST(l.total_cost_usd AS REAL), l.latency_ms, l.first_token_ms, \
                    l.duration_ms, l.status_code, l.error_message, COALESCE(l.data_source,'proxy') \
             FROM proxy_request_logs l LEFT JOIN providers p ON p.id = l.provider_id AND p.app_type = l.app_type \
             WHERE {w}{status_sql} ORDER BY l.created_at DESC LIMIT ? OFFSET ?",
            provider_name_sql("l", "p"),
            fresh_input_sql("l"),
        );
        let mut row_params: Vec<&dyn ToSql> = where_params;
        row_params.push(&status);
        row_params.push(&status);
        row_params.push(&limit);
        row_params.push(&offset);

        let mut stmt = self.conn.prepare(&rows_sql)?;
        let mut q = stmt.query(row_params.as_slice())?;
        let mut rows = Vec::new();
        while let Some(r) = q.next()? {
            rows.push(RequestLog {
                request_id: r.get(0)?,
                created_at: r.get(1)?,
                app_type: r.get(2)?,
                provider_id: r.get(3)?,
                provider_name: r.get(4)?,
                model: r.get(5)?,
                request_model: r.get(6)?,
                pricing_model: r.get(7)?,
                fresh: r.get(8)?,
                cached: r.get(9)?,
                ccreate: r.get(10)?,
                output: r.get(11)?,
                cost: r.get(12)?,
                latency_ms: r.get(13)?,
                first_token_ms: r.get(14)?,
                duration_ms: r.get(15)?,
                status_code: r.get(16)?,
                error_message: r.get(17)?,
                data_source: r.get(18)?,
            });
        }
        Ok(RequestPage { rows, total })
    }

    // ---- filter options (only values present under current filters) ----
    pub fn filter_options(&self, filter: &UsageFilter) -> Result<FilterOptions> {
        let agent_filter = UsageFilter { range: filter.range, ..Default::default() };
        let agents = self.distinct_column(
            &agent_filter,
            &format!("{} c", display_app_sql("l")),
            &format!("{} c", display_app_sql("r")),
        )?;

        let prov_filter = UsageFilter { range: filter.range, agent: filter.agent.clone(), ..Default::default() };
        let providers = self.distinct_providers(&prov_filter)?;

        let model_filter = UsageFilter {
            range: filter.range,
            agent: filter.agent.clone(),
            provider: filter.provider.clone(),
            ..Default::default()
        };
        let models = self.distinct_column(
            &model_filter,
            &format!("{} c", effective_model_sql("l")),
            &format!("{} c", effective_model_sql("r")),
        )?;

        Ok(FilterOptions { agents, providers, models })
    }

    fn distinct_column(&self, filter: &UsageFilter, raw_expr: &str, roll_expr: &str) -> Result<Vec<String>> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };
        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!("SELECT {raw_expr} FROM proxy_request_logs l WHERE {w}"));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!("SELECT {roll_expr} FROM usage_daily_rollups r WHERE {w}"));
        }
        let sql = format!(
            "SELECT DISTINCT c FROM ( {} ) WHERE c IS NOT NULL ORDER BY c",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push(r.get::<_, String>(0)?);
        }
        Ok(out)
    }

    fn distinct_providers(&self, filter: &UsageFilter) -> Result<Vec<ProviderKey>> {
        let bounds = filter.range.bounds();
        let pid = filter.provider.as_ref().map(|p| p.provider_id.clone());
        let papp = filter.provider.as_ref().map(|p| p.app_type.clone());
        let ctx = Ctx { filter, pid: &pid, papp: &papp, bounds: &bounds };
        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut parts = Vec::new();
        if bounds.include_raw {
            let w = build_where(&ctx, "l", true, &mut params);
            parts.push(format!(
                "SELECT l.app_type app, l.provider_id pid, {} pname \
                 FROM proxy_request_logs l LEFT JOIN providers p ON p.id=l.provider_id AND p.app_type=l.app_type \
                 WHERE {w}",
                provider_name_sql("l", "p")
            ));
        }
        if bounds.include_rollups {
            let w = build_where(&ctx, "r", false, &mut params);
            parts.push(format!(
                "SELECT r.app_type app, r.provider_id pid, {} pname \
                 FROM usage_daily_rollups r LEFT JOIN providers p ON p.id=r.provider_id AND p.app_type=r.app_type \
                 WHERE {w}",
                provider_name_sql("r", "p")
            ));
        }
        let sql = format!(
            "SELECT app, pid, MAX(pname) FROM ( {} ) GROUP BY app, pid ORDER BY MAX(pname)",
            parts.join(" UNION ALL ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut q = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(r) = q.next()? {
            out.push(ProviderKey { app_type: r.get(0)?, provider_id: r.get(1)?, display_name: r.get(2)? });
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// local-time formatting helpers
// ---------------------------------------------------------------------------

fn fmt_hour_local(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(d) => d.format("%H:%M").to_string(),
        None => "--:--".into(),
    }
}

fn date_str_to_local_ts(d: &str) -> i64 {
    NaiveDate::parse_from_str(d, "%Y-%m-%d").map(local_midnight_ts).unwrap_or(0)
}

fn short_date(d: &str) -> String {
    d.get(5..).unwrap_or(d).to_string()
}

fn fill_hourly_gaps(pts: Vec<TrendPoint>, end: i64) -> Vec<TrendPoint> {
    if pts.is_empty() {
        return pts;
    }
    let first = pts[0].ts;
    let last_bucket = end - (end % 3600);
    let mut out = Vec::new();
    let mut idx = 0;
    let mut t = first;
    while t <= last_bucket.max(first) && out.len() <= 48 {
        if idx < pts.len() && pts[idx].ts == t {
            out.push(pts[idx].clone());
            idx += 1;
        } else {
            out.push(TrendPoint { ts: t, label: fmt_hour_local(t), ..Default::default() });
        }
        t += 3600;
    }
    out
}

/// Minimal schema for tests (only the columns the queries touch).
pub const TEST_SCHEMA: &str = r#"
CREATE TABLE providers (
    id TEXT NOT NULL,
    app_type TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (id, app_type)
);
CREATE TABLE proxy_request_logs (
    request_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    app_type TEXT NOT NULL,
    model TEXT NOT NULL,
    request_model TEXT,
    pricing_model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    input_token_semantics INTEGER NOT NULL DEFAULT 0,
    total_cost_usd TEXT NOT NULL DEFAULT '0',
    latency_ms INTEGER NOT NULL DEFAULT 0,
    first_token_ms INTEGER,
    duration_ms INTEGER,
    status_code INTEGER NOT NULL DEFAULT 200,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    data_source TEXT NOT NULL DEFAULT 'proxy'
);
CREATE TABLE usage_daily_rollups (
    date TEXT NOT NULL,
    app_type TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    request_model TEXT NOT NULL DEFAULT '',
    pricing_model TEXT NOT NULL DEFAULT '',
    request_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost_usd TEXT NOT NULL DEFAULT '0',
    avg_latency_ms INTEGER NOT NULL DEFAULT 0,
    input_token_semantics INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
);
"#;
