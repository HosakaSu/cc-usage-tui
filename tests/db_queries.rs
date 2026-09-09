//! Integration tests for the cc-switch usage query layer.
//! Uses an in-memory SQLite DB with a minimal schema and synthetic fixtures to
//! verify every normalization/dedup/filter/range rule.

use cc_usage_tui::db::*;
use chrono::Local;
use rusqlite::{params, Connection};

fn mem() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(TEST_SCHEMA).unwrap();
    c
}

#[allow(clippy::too_many_arguments)]
fn raw(
    c: &Connection,
    id: &str,
    app: &str,
    pid: &str,
    model: &str,
    pricing: Option<&str>,
    input: i64,
    cread: i64,
    ccreate: i64,
    out: i64,
    sem: i64,
    cost: &str,
    status: i64,
    ts: i64,
    src: &str,
) {
    c.execute(
        "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model, request_model, \
         pricing_model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, \
         input_token_semantics, total_cost_usd, latency_ms, first_token_ms, duration_ms, \
         status_code, error_message, created_at, data_source) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        params![
            id,
            pid,
            app,
            model,
            None::<String>,
            pricing,
            input,
            out,
            cread,
            ccreate,
            sem,
            cost,
            100i64,
            None::<i64>,
            None::<i64>,
            status,
            None::<String>,
            ts,
            src
        ],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn roll(
    c: &Connection,
    date: &str,
    app: &str,
    pid: &str,
    model: &str,
    pricing: &str,
    reqcount: i64,
    succ: i64,
    input: i64,
    out: i64,
    cread: i64,
    ccreate: i64,
    cost: &str,
    avg_lat: i64,
    sem: i64,
) {
    c.execute(
        "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_model, \
         pricing_model, request_count, success_count, input_tokens, output_tokens, cache_read_tokens, \
         cache_creation_tokens, total_cost_usd, avg_latency_ms, input_token_semantics) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            date, app, pid, model, "", pricing, reqcount, succ, input, out, cread, ccreate, cost,
            avg_lat, sem
        ],
    )
    .unwrap();
}

fn prov(c: &Connection, id: &str, app: &str, name: &str) {
    c.execute(
        "INSERT INTO providers (id, app_type, name) VALUES (?1,?2,?3)",
        params![id, app, name],
    )
    .unwrap();
}

fn all() -> UsageFilter {
    UsageFilter { range: Range::All, ..Default::default() }
}

// ---------------------------------------------------------------------------
// fresh-input semantics
// ---------------------------------------------------------------------------

#[test]
fn codex_semantics() {
    let c = mem();
    // sem=0 (legacy inclusive): fresh = input - cache_read
    raw(&c, "a", "codex", "_codex_session", "gpt", None, 1000, 800, 0, 10, 0, "0.1", 200, 1, "codex_session");
    // sem=1 (inclusive): fresh = input - cache_read - cache_creation
    raw(&c, "b", "codex", "_codex_session", "gpt", None, 1000, 700, 100, 10, 1, "0.1", 200, 2, "codex_session");
    // sem=2 (exclusive): fresh = input
    raw(&c, "c", "codex", "_codex_session", "gpt", None, 1000, 800, 0, 10, 2, "0.1", 200, 3, "codex_session");
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    // 200 + 200 + 1000 = 1400
    assert_eq!(s.fresh, 1400, "codex fresh across sem 0/1/2");
    assert_eq!(s.cached, 800 + 700 + 800);
    assert_eq!(s.reqs, 3);
}

#[test]
fn gemini_is_cache_inclusive() {
    let c = mem();
    raw(&c, "g1", "gemini", "_gemini_session", "gem", None, 1000, 600, 100, 5, 1, "0.1", 200, 1, "gemini_session");
    raw(&c, "g0", "gemini", "_gemini_session", "gem", None, 1000, 600, 0, 5, 0, "0.1", 200, 2, "gemini_session");
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    // sem1: 1000-600-100=300 ; sem0: 1000-600=400  => 700
    assert_eq!(s.fresh, 700);
}

#[test]
fn claude_is_fresh_only() {
    let c = mem();
    // claude sem=0 with cache_read > input must NOT go negative; fresh = input
    raw(&c, "cl", "claude", "_session", "opus", None, 500, 800, 0, 20, 0, "0.2", 200, 1, "claude_session");
    raw(&c, "c2", "claude", "_session", "opus", None, 500, 800, 0, 20, 2, "0.2", 200, 2, "claude_session");
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    assert_eq!(s.fresh, 1000, "claude fresh = input for both sem 0 and 2");
    assert_eq!(s.cached, 1600);
}

#[test]
fn inclusive_negative_guard() {
    let c = mem();
    // codex sem=0 but cache_read > input => guard fails => ELSE => fresh = input
    raw(&c, "x", "codex", "_codex_session", "gpt", None, 100, 800, 0, 5, 0, "0.1", 200, 1, "codex_session");
    let rd = Reader::from_connection(c);
    assert_eq!(rd.summary(&all()).unwrap().fresh, 100);
}

// ---------------------------------------------------------------------------
// display app / effective model / provider name
// ---------------------------------------------------------------------------

#[test]
fn claude_desktop_folds_into_claude() {
    let c = mem();
    raw(&c, "1", "claude", "_session", "opus", None, 10, 0, 0, 1, 2, "1.0", 200, 1, "s");
    raw(&c, "2", "claude-desktop", "_session", "opus", None, 10, 0, 0, 1, 2, "1.0", 200, 2, "s");
    let rd = Reader::from_connection(c);
    let agents = rd.agent_stats(&all()).unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent, "claude");
    assert_eq!(agents[0].reqs, 2);
}

#[test]
fn effective_model_prefers_pricing_model() {
    let c = mem();
    raw(&c, "1", "codex", "p", "gpt-raw", Some("gpt-priced"), 10, 0, 0, 1, 2, "1.0", 200, 1, "s");
    raw(&c, "2", "codex", "p", "gpt-raw2", Some(""), 10, 0, 0, 1, 2, "1.0", 200, 2, "s");
    raw(&c, "3", "codex", "p", "gpt-raw3", None, 10, 0, 0, 1, 2, "1.0", 200, 3, "s");
    let rd = Reader::from_connection(c);
    let models = rd.model_stats(&all()).unwrap();
    let names: Vec<&str> = models.iter().map(|m| m.model.as_str()).collect();
    assert!(names.contains(&"gpt-priced"), "pricing_model wins: {names:?}");
    assert!(names.contains(&"gpt-raw2"), "empty pricing falls back to model");
    assert!(names.contains(&"gpt-raw3"), "null pricing falls back to model");
}

#[test]
fn provider_name_fallback_and_join() {
    let c = mem();
    prov(&c, "real-1", "codex", "OpenAI Official");
    raw(&c, "1", "codex", "real-1", "gpt", None, 10, 0, 0, 1, 2, "1.0", 200, 1, "s");
    raw(&c, "2", "codex", "_codex_session", "gpt", None, 10, 0, 0, 1, 2, "1.0", 200, 2, "s");
    raw(&c, "3", "claude", "_session", "opus", None, 10, 0, 0, 1, 2, "1.0", 200, 3, "s");
    raw(&c, "4", "pi", "_weird", "q", None, 10, 0, 0, 1, 2, "1.0", 200, 4, "s");
    let rd = Reader::from_connection(c);
    let ps = rd.provider_stats(&all()).unwrap();
    let find = |id: &str| ps.iter().find(|p| p.provider_id == id).unwrap().provider_name.clone();
    assert_eq!(find("real-1"), "OpenAI Official");
    assert_eq!(find("_codex_session"), "Codex (Session)");
    assert_eq!(find("_session"), "Claude (Session)");
    assert_eq!(find("_weird"), "_weird", "unknown id falls back to raw id");
}

// ---------------------------------------------------------------------------
// raw + rollup merge, ranges
// ---------------------------------------------------------------------------

#[test]
fn raw_plus_rollup_no_double_count() {
    let c = mem();
    let now = Local::now().timestamp();
    raw(&c, "1", "codex", "_codex_session", "gpt", None, 100, 0, 0, 10, 2, "1.0", 200, now, "s");
    // rollup for an old day (no overlap with raw)
    roll(&c, "2020-01-01", "codex", "_codex_session", "gpt", "", 5, 5, 500, 50, 0, 0, "2.5", 10, 2);
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    assert_eq!(s.reqs, 6, "1 raw + 5 rollup");
    assert_eq!(s.fresh, 600, "100 raw + 500 rollup");
    assert!((s.cost - 3.5).abs() < 1e-9);
}

#[test]
fn live_range_excludes_rollups() {
    let c = mem();
    let now = Local::now().timestamp();
    raw(&c, "1", "codex", "_codex_session", "gpt", None, 100, 0, 0, 10, 2, "1.0", 200, now, "s");
    roll(&c, "2020-01-01", "codex", "_codex_session", "gpt", "", 5, 5, 500, 50, 0, 0, "2.5", 10, 2);
    let rd = Reader::from_connection(c);
    let live = UsageFilter { range: Range::Live, ..Default::default() };
    let s = rd.summary(&live).unwrap();
    assert_eq!(s.reqs, 1, "Live = raw only");
    assert_eq!(s.fresh, 100);
}

#[test]
fn days_range_includes_recent_rollup() {
    let c = mem();
    let now = Local::now();
    let yday = (now - chrono::Duration::days(1)).date_naive().to_string();
    // rollup dated yesterday should be inside a 7d window
    roll(&c, &yday, "codex", "_codex_session", "gpt", "", 3, 3, 300, 30, 0, 0, "1.5", 10, 2);
    // rollup dated 60 days ago should be outside a 7d window
    let old = (now - chrono::Duration::days(60)).date_naive().to_string();
    roll(&c, &old, "codex", "_codex_session", "gpt", "", 9, 9, 900, 90, 0, 0, "9.0", 10, 2);
    let rd = Reader::from_connection(c);
    let d7 = UsageFilter { range: Range::Days(7), ..Default::default() };
    let s = rd.summary(&d7).unwrap();
    assert_eq!(s.reqs, 3, "only yesterday's rollup within 7d");
    assert_eq!(s.fresh, 300);
}

// ---------------------------------------------------------------------------
// dedup (proxy vs session for the same call)
// ---------------------------------------------------------------------------

#[test]
fn proxy_session_duplicate_counted_once() {
    let c = mem();
    let ts = 1_700_000_000;
    // same fingerprint (app, tokens, created_at), different source + id
    raw(&c, "proxy-1", "codex", "_codex_session", "gpt", None, 1000, 800, 0, 10, 0, "0.5", 200, ts, "proxy");
    raw(&c, "sess-1", "codex", "_codex_session", "gpt", None, 1000, 800, 0, 10, 0, "0.5", 200, ts, "codex_session");
    // a distinct call (different ts) must NOT be deduped
    raw(&c, "sess-2", "codex", "_codex_session", "gpt", None, 1000, 800, 0, 10, 0, "0.5", 200, ts + 5, "codex_session");
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    assert_eq!(s.reqs, 2, "duplicate pair collapses to 1, plus the distinct call");
}

// ---------------------------------------------------------------------------
// requests: pagination + status filter
// ---------------------------------------------------------------------------

#[test]
fn requests_pagination_and_status() {
    let c = mem();
    let base = 1_700_000_000;
    for i in 0..5 {
        let status = if i % 2 == 0 { 200 } else { 500 };
        raw(&c, &format!("r{i}"), "codex", "_codex_session", "gpt", None, 100, 0, 0, 1, 2, "0.1", status, base + i, "s");
    }
    let rd = Reader::from_connection(c);

    let p0 = rd.request_logs(&all(), None, 0, 2).unwrap();
    assert_eq!(p0.total, 5);
    assert_eq!(p0.rows.len(), 2);
    // newest first
    assert_eq!(p0.rows[0].request_id, "r4");

    let p2 = rd.request_logs(&all(), None, 2, 2).unwrap();
    assert_eq!(p2.rows.len(), 1);
    assert_eq!(p2.rows[0].request_id, "r0");

    let errs = rd.request_logs(&all(), Some(500), 0, 10).unwrap();
    assert_eq!(errs.total, 2);
    assert!(errs.rows.iter().all(|r| r.status_code == 500));
}

// ---------------------------------------------------------------------------
// filters + cascading options
// ---------------------------------------------------------------------------

#[test]
fn agent_provider_model_filters() {
    let c = mem();
    prov(&c, "openai", "codex", "OpenAI");
    raw(&c, "1", "codex", "openai", "gpt-a", None, 100, 0, 0, 1, 2, "1.0", 200, 10, "s");
    raw(&c, "2", "codex", "openai", "gpt-b", None, 100, 0, 0, 1, 2, "1.0", 200, 11, "s");
    raw(&c, "3", "opencode", "_opencode_session", "glm", None, 100, 0, 0, 1, 2, "1.0", 200, 12, "s");
    let rd = Reader::from_connection(c);

    // agent filter
    let f = UsageFilter { range: Range::All, agent: Some("codex".into()), ..Default::default() };
    assert_eq!(rd.summary(&f).unwrap().reqs, 2);

    // model filter
    let f = UsageFilter { range: Range::All, model: Some("gpt-a".into()), ..Default::default() };
    assert_eq!(rd.summary(&f).unwrap().reqs, 1);

    // provider filter
    let pk = ProviderKey { app_type: "codex".into(), provider_id: "openai".into(), display_name: "OpenAI".into() };
    let f = UsageFilter { range: Range::All, provider: Some(pk), ..Default::default() };
    assert_eq!(rd.summary(&f).unwrap().reqs, 2);
}

#[test]
fn filter_options_cascade() {
    let c = mem();
    prov(&c, "openai", "codex", "OpenAI");
    raw(&c, "1", "codex", "openai", "gpt-a", None, 100, 0, 0, 1, 2, "1.0", 200, 10, "s");
    raw(&c, "2", "opencode", "_opencode_session", "glm", None, 100, 0, 0, 1, 2, "1.0", 200, 11, "s");
    let rd = Reader::from_connection(c);

    let opts_all = rd.filter_options(&all()).unwrap();
    assert!(opts_all.agents.contains(&"codex".to_string()));
    assert!(opts_all.agents.contains(&"opencode".to_string()));

    // constrain to codex -> providers/models only for codex
    let f = UsageFilter { range: Range::All, agent: Some("codex".into()), ..Default::default() };
    let opts = rd.filter_options(&f).unwrap();
    assert_eq!(opts.providers.len(), 1);
    assert_eq!(opts.providers[0].display_name, "OpenAI");
    assert_eq!(opts.models, vec!["gpt-a".to_string()]);
}

// ---------------------------------------------------------------------------
// summary success rate
// ---------------------------------------------------------------------------

#[test]
fn summary_success_rate() {
    let c = mem();
    raw(&c, "1", "codex", "p", "m", None, 10, 0, 0, 1, 2, "0.1", 200, 1, "s");
    raw(&c, "2", "codex", "p", "m", None, 10, 0, 0, 1, 2, "0.1", 200, 2, "s");
    raw(&c, "3", "codex", "p", "m", None, 10, 0, 0, 1, 2, "0.1", 500, 3, "s");
    let rd = Reader::from_connection(c);
    let s = rd.summary(&all()).unwrap();
    assert_eq!(s.reqs, 3);
    assert_eq!(s.success, 2);
    assert!((s.success_rate() - 2.0 / 3.0).abs() < 1e-9);
}
