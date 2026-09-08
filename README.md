# cc-usage-tui

A **read-only**, near-realtime terminal dashboard for [cc-switch](https://github.com/farion1231/cc-switch)'s
token/cost usage. It tails cc-switch's SQLite database and shows, per agent tool
(codex, opencode, claude, pi, grokbuild, …), the **real** consumption:

- requests
- fresh (non-cached) input tokens
- cache-read tokens
- output tokens
- total tokens
- **cache hit rate**
- total cost (USD)

```
┌ cc-switch usage ───────────────────────────────────────────────────────────────┐
│DB C:\Users\you\.cc-switch\cc-switch.db                                          │
│journal DELETE  ·  2.64 MB  ·   READ-ONLY   ·  rows raw 3,321 / rollups 55       │
│last write 06:50:58 UTC (2m ago)  ·  refreshed 06:53:02 UTC (1x)                 │
│range All history  ·  view Agents  ·  sort Cost  ·  [ok]                         │
└─────────────────────────────────────────────────────────────────────────────────┘
┌ Usage by agent ────────────────────────────────────────────────────────────────┐
│  Agent                    Reqs   Fresh In  Cache Read    Output   Total Tok  Hit %       Cost│
│▸ codex                   3,707     12.65M     442.52M     1.44M     456.61M   97.2%     $327.60│
│  opencode                2,290     15.88M     146.61M     2.49M     164.98M   90.2%      $57.47│
│  claude                  1,197      2.25M     112.14M    646.7K     117.40M   96.1%      $38.48│
│  ...                                                                       │
│  TOTAL                    7,885     38.71M     708.07M     4.78M     753.91M   94.5%     $424.22│
└─────────────────────────────────────────────────────────────────────────────────┘
 q quit   r refresh   t range   v view   s sort   ↑↓/jk select   (auto-refreshes on each write)
```

## Why read-only, and how it stays safe

cc-switch keeps `cc-switch.db` open and writes to it in realtime (one row per API
request). This tool **never writes**:

- Opens the file with `SQLITE_OPEN_READ_ONLY` — it cannot create `-wal`/`-journal`
  files or mutate anything.
- Sets a per-connection `busy_timeout` (2 s) so that if cc-switch holds the write
  lock at that instant, the read simply waits instead of failing with
  `SQLITE_BUSY`. cc-switch uses `journal_mode=DELETE`, where readers and writers
  are mutually exclusive, so this matters.
- On any lock/error it keeps the last good data and shows `[db busy — retrying…]`
  in the status line rather than crashing.

**Refresh model:** the event loop polls the file's mtime every 200 ms; when it
changes (i.e. cc-switch wrote), it re-queries — so the view updates right after
each write. A 5 s fallback refresh covers the rare case where mtime is unreliable.

## Data model (why the numbers are "real")

cc-switch stores usage in two complementary, non-overlapping tables:

| table | contents | window |
|---|---|---|
| `proxy_request_logs` | per-request rows (live) | recent (~last weeks) |
| `usage_daily_rollups` | pre-aggregated daily rows (archived) | older |

Old rows are rolled up and pruned, so summing both gives full history with **no
double counting** (the tool asserts the ranges don't overlap by construction).

Token accounting differs per source and is normalized via `input_token_semantics`:

- `1` → `input_tokens` **already includes** `cache_read` (cache-inclusive)
- `2` → `input_tokens` **excludes** `cache_read` (fresh only)
- `0` → unknown; decided by agent — **codex/grokbuild** report cache-inclusive,
  **opencode/pi/claude** report fresh-only.

Everything is normalized into `fresh`, `cache_read`, `cache_creation`, `output`,
so totals and cache-hit rate are consistent across agents.
`cache_hit = cache_read / (fresh + cache_read + cache_creation)`.
Cost is taken from cc-switch's own `total_cost_usd` (not recomputed).

## Build

Requires Rust (MSVC toolchain on Windows). `rusqlite` compiles a bundled SQLite,
which needs a C compiler, so on Windows build through the MSVC environment:

```bat
build.bat
```

`build.bat` locates Visual Studio / Build Tools via `vswhere`, enters `vcvars64`,
and runs `cargo build --release`. Binary: `target\release\cc-usage-tui.exe`.

On macOS/Linux (system or bundled SQLite both fine):

```sh
cargo build --release
```

## Usage

```
cc-usage-tui [DB_PATH] [OPTIONS]
```

- `DB_PATH` defaults to `%USERPROFILE%\.cc-switch\cc-switch.db` (or `$HOME/.cc-switch/cc-switch.db`).

Options:

| flag | meaning |
|---|---|
| *(none)* | launch the interactive TUI |
| `--dump` | print a one-shot text report and exit (no TUI) |
| `--frame` | render a single TUI frame as text and exit (no tty needed; great for screenshots/CI) |
| `--by-model` | break down by model instead of by agent (with `--dump`/`--frame`) |
| `--range <R>` | `all` (default) · `live` (raw logs only) · `today` · `7` · `30` |
| `--interval <ms>` | mtime poll interval for write detection (default 200) |

Examples:

```bat
:: live dashboard on the default db
cc-usage-tui

:: text report of everything, per agent
cc-usage-tui --dump

:: per-model breakdown of the last 7 days
cc-usage-tui --dump --by-model --range 7

:: analyze a specific backup snapshot
cc-usage-tui --dump "C:\Users\you\.cc-switch\backups\db_backup_YYYYMMDD_HHMMSS.db"
```

TUI keybindings:

| key | action |
|---|---|
| `q` / `Esc` / `Ctrl-C` | quit |
| `r` | refresh now |
| `t` | cycle range (All → Live → 30d → 7d → Today) |
| `v` | toggle Agents ↔ Models view |
| `s` | cycle sort (Cost → Total tokens → Requests → Cache hit → Name) |
| `↑`/`↓` or `k`/`j` | move selection |

## Reuse / credits

Built on existing wheels rather than reinventing:

- [ratatui](https://crates.io/crates/ratatui) `0.29` — TUI framework (pinned to
  0.29's stable API; 0.30 is a fresh modular rewrite).
- [crossterm](https://crates.io/crates/crossterm) `0.28` — terminal backend/input.
- [rusqlite](https://crates.io/crates/rusqlite) `0.40` (`bundled`) — SQLite.
- [anyhow](https://crates.io/crates/anyhow) — errors.

Conceptual prior art: [`ccusage`](https://github.com/ryoppippi/ccusage) (a
TypeScript CLI that visualizes Claude Code usage) — but it reads `~/.claude`
JSONL logs, not cc-switch's SQLite, so no existing tool fit this data source.

## Notes / limitations

- Times are shown in UTC (matching cc-switch's epoch `created_at`).
- If cc-switch ever changes its schema or token semantics, adjust the
  normalization in `src/db.rs` (`is_cache_inclusive` / `fresh_input`).
- `cache_creation` tokens are folded into totals and hit-rate; they are currently
  zero for all sources in this database.
