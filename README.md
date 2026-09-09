# cc-usage-tui

A **read-only**, near-realtime terminal dashboard for [cc-switch](https://github.com/farion1231/cc-switch)'s
token/cost usage. It tails cc-switch's SQLite database and, after each write,
shows the **real** consumption per agent tool / provider / model: requests,
fresh (non-cached) input, cache-read, output, total tokens, **cache-hit rate**,
cost, and success rate.

Five pages, one global filter that drives all of them:

| page | shows |
|---|---|
| **Overview** | usage by agent (codex, opencode, claude, pi, grokbuild, …) |
| **Trend** | time-series chart — Tokens (fresh/cache/output), Cost, or Requests; hourly for Today, daily otherwise |
| **Requests** | paginated per-request log + detail pane (latency, first-token, source, error…) |
| **Providers** | usage by provider (with `providers.name`, session fallbacks, success %, avg latency) |
| **Models** | usage by **effective pricing model** (`COALESCE(NULLIF(pricing_model,''), model)`) with avg tokens/req |

```
Requests 7,966   Tokens 768.18M   Hit 94.6%   Cost $428.98   Success 99.1%
DB C:\Users\you\.cc-switch\cc-switch.db · DELETE · 2.64MB · READ-ONLY · last write 16:21:30 (40s ago)
status [ok] · filters affect every page · local-time ranges
  Overview   Trend   Requests  [Providers]  Models
Range: [All]  Agent: [All]  Provider: [All]  Model: [All]
┌ Usage by provider ───────────────────────────────────────────────────────────┐
│  Provider              Agent        Reqs    Tokens      Cost  Success  Avg Lat│
│▸ Codex (Session)       codex       3,712   456.71M   $327.92   100.0%     0ms│
│  PackyCode             claude        309    26.80M    $22.29    99.4%   14.21s│
│  ...                                                                         │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Read-only & refresh model (unchanged guarantees)

- Opens with `SQLITE_OPEN_READ_ONLY` — never writes, never creates `-wal`/`-journal`.
- `busy_timeout=2s` so a read waits instead of failing when cc-switch holds the
  write lock (cc-switch uses `journal_mode=DELETE`, where readers/writers are
  mutually exclusive). On any error it keeps the last good data and shows
  `[db busy — retrying…]`.
- Event loop polls the file **mtime every 200 ms**; on change it re-queries, so the
  view updates right after each cc-switch write (plus a 5 s fallback).
- **Page-aware refresh**: each refresh runs only `meta + summary + current page`
  queries (not every page), and `filter_options` only reloads when a filter or the
  range changes — so one write stays cheap.
- Never creates indexes or mutates cc-switch's schema.

## Data model — aligned with cc-switch's Dashboard

cc-switch keeps two complementary, **non-overlapping** tables (raw rows are pruned
once rolled up):

| table | contents | window |
|---|---|---|
| `proxy_request_logs` | per-request rows (live) | recent |
| `usage_daily_rollups` | pre-aggregated daily rows | older |

All aggregation SQL mirrors cc-switch's own conventions so the numbers match its
web UI (see `src/db.rs`):

- **Fresh input** via `input_token_semantics` + agent rules:
  `sem=2 → input`; `sem=1` (cache-inclusive agents) `→ input − read − create`;
  `sem=0` (cache-inclusive agents) `→ input − read`; else `→ input`.
  Cache-inclusive agents: **codex, gemini, grokbuild**.
- **Effective model** = `COALESCE(NULLIF(pricing_model,''), model)`.
- **Display agent**: `claude-desktop → claude` (details still keep the real app_type).
- **Provider name**: `providers.name`, else a `(Session)` fallback for
  `_session/_codex_session/_gemini_session/_opencode_session/_grok_session/_pi_session`,
  else the raw id.
- **Cross-source dedup** (proxy vs session for the same call) via a fingerprint
  window — a **no-op** when only one source exists (current cc-switch data is
  session-only), present for parity with upstream.
- **Time ranges use LOCAL dates** (chrono), and ranges merge raw detail +
  full-day rollups, so `7d`/`30d` still see history after raw rows are pruned.
  `Live` is the only raw-only range.

## Build

Rust (MSVC toolchain on Windows). `rusqlite` compiles a bundled SQLite, which
needs a C compiler, so on Windows build through the MSVC environment:

```bat
build.bat
```

`build.bat` locates Visual Studio / Build Tools via `vswhere`, enters `vcvars64`,
and runs `cargo build --release` → `target\release\cc-usage-tui.exe`.

macOS/Linux:

```sh
cargo build --release
```

## Usage

```
cc-usage-tui [DB_PATH] [OPTIONS]
```

- `DB_PATH` defaults to `%USERPROFILE%\.cc-switch\cc-switch.db` (`$HOME/.cc-switch/cc-switch.db`).

| flag | meaning |
|---|---|
| *(none)* | launch the interactive TUI |
| `--dump` | one-shot text report and exit (no TUI) |
| `--by-model` | with `--dump`, report by model instead of agent |
| `--frame` | render one TUI frame as text and exit (no tty; good for screenshots/CI) |
| `--page <P>` | with `--frame`: `overview\|trend\|requests\|providers\|models` |
| `--range <R>` | `all` (default) · `live` · `today` · `7` · `30` |
| `--interval <ms>` | mtime poll interval (default 200) |

Examples:

```bat
cc-usage-tui                                  :: live dashboard (default db)
cc-usage-tui --dump                           :: text report, per agent
cc-usage-tui --dump --by-model --range 7      :: per-model, last 7 days
cc-usage-tui --frame --page trend             :: text screenshot of the Trend page
cc-usage-tui --dump "…\backups\db_backup_YYYYMMDD_HHMMSS.db"   :: analyze a snapshot
```

### Keys

| key | action |
|---|---|
| `q` / `Esc` / `Ctrl-C` | quit |
| `r` | refresh now (also reloads filter options) |
| `Tab` / `1`–`5` / `←` `→` | switch page |
| `t` | cycle range (All → Live → 30d → 7d → Today) |
| `a` / `p` / `m` | open agent / provider / model picker (cascading) |
| `s` | cycle sort (Cost → Tokens → Reqs → Hit%/Success → Name) |
| `↑`/`↓` or `k`/`j`, `PgUp`/`PgDn`, `Home`/`End` | move selection |
| `Enter` / click a row | drill down (set that agent/model/provider as filter) |
| Trend: `x` | cycle metric (Tokens/Cost/Requests) |
| Requests: `f`, `n`/`b` | cycle status filter, next/prev page |
| picker: `↑`/`↓`, `Enter`, `Esc` | choose / confirm / cancel |

### Mouse

Left-click tabs, filter chips, and table rows; wheel scrolls the selection.
(Only left-click + wheel are handled to avoid event spam.)

## Architecture

```
src/
├── main.rs        CLI, terminal lifecycle, event loop (no business logic)
├── lib.rs         exposes the modules (so tests/ can use them)
├── app.rs         App/Page/UsageFilter/UiAction + per-page state + apply()/refresh
├── input.rs       keyboard + mouse → UiAction (one path for both)
├── db.rs          read-only Reader, SQL helpers, all queries
└── ui/
    ├── mod.rs     layout + page router + hit-region collection
    ├── common.rs  formatting, header/tabs/filter bar/footer/popup
    ├── overview.rs  trend.rs  requests.rs  stats.rs
tests/
└── db_queries.rs  15 tests over an in-memory SQLite: semantics, dedup, ranges,
                   provider fallback, effective model, pagination, filters
```

Data + control flow: `keyboard/mouse → UiAction → App::apply() → Reader (read-only)`.
UI is a pure projection of `App`; hit regions are rebuilt every frame so mouse
coordinates survive resizes.

Run the tests:

```sh
cargo test          # 15 passed
```

## Reuse / credits

- [ratatui](https://crates.io/crates/ratatui) `0.29` (0.30 is a fresh modular rewrite),
  [crossterm](https://crates.io/crates/crossterm) `0.28`,
  [rusqlite](https://crates.io/crates/rusqlite) `0.40` (`bundled`),
  [chrono](https://crates.io/crates/chrono) `0.4`,
  [anyhow](https://crates.io/crates/anyhow) `1`.
- Prior art: [`ccusage`](https://github.com/ryoppippi/ccusage) (TypeScript, reads
  Claude Code JSONL — a different data source), so no existing tool fit cc-switch's
  SQLite; the SQL conventions here mirror cc-switch's own Dashboard.

## Notes / limitations

- Times are shown in **local** time (matching cc-switch's local-date Dashboard).
- If cc-switch changes its schema/token semantics, adjust the helpers in `src/db.rs`.
- `cache_creation` is folded into totals/hit-rate (currently zero for all sources).
- Mouse header-click sorting is not wired; use `s` to cycle sort.
