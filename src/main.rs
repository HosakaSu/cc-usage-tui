use cc_usage_tui::app::{App, Page};
use cc_usage_tui::db::{Range, Reader, UsageFilter};
use cc_usage_tui::ui::common::{human_cost, human_int, human_tokens, local_hms_of, pct};
use cc_usage_tui::{input, ui};

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path: Option<PathBuf> = None;
    let mut want_dump = false;
    let mut frame_mode = false;
    let mut by_model = false;
    let mut range = Range::All;
    let mut page = Page::Overview;
    let mut interval_ms: u64 = 200;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => want_dump = true,
            "--frame" => frame_mode = true,
            "--by-model" => by_model = true,
            "-h" | "--help" => {
                print_help();
                return;
            }
            "--range" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    range = parse_range(v);
                }
            }
            "--page" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    page = parse_page(v);
                }
            }
            "--interval" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    interval_ms = v.parse().unwrap_or(200);
                }
            }
            other => {
                if let Some(v) = other.strip_prefix("--range=") {
                    range = parse_range(v);
                } else if let Some(v) = other.strip_prefix("--page=") {
                    page = parse_page(v);
                } else if !other.starts_with('-') {
                    path = Some(PathBuf::from(other));
                }
            }
        }
        i += 1;
    }

    let path = path.unwrap_or_else(default_db_path);
    if !path.exists() {
        eprintln!("error: database not found: {}", path.display());
        eprintln!("hint: pass a path, e.g.  cc-usage-tui \"C:\\Users\\<you>\\.cc-switch\\cc-switch.db\"");
        std::process::exit(1);
    }
    let reader = match Reader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    };

    let filter = UsageFilter { range, ..Default::default() };
    let result = if want_dump {
        dump(&reader, &filter, by_model)
    } else if frame_mode {
        render_frame(&reader, &filter, page)
    } else {
        run_tui(&reader, &path, filter, interval_ms)
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "cc-usage-tui — read-only realtime dashboard for cc-switch\n\
         \n\
         USAGE:\n    cc-usage-tui [DB_PATH] [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \x20   --dump            print a one-shot text report and exit (no TUI)\n\
         \x20   --frame           render one TUI frame as text and exit (no tty needed)\n\
         \x20   --by-model        with --dump, report by model instead of agent\n\
         \x20   --range <R>       all | live | today | 7 | 30   (default: all)\n\
         \x20   --page <P>        overview|trend|requests|providers|models (with --frame)\n\
         \x20   --interval <ms>   poll interval for write detection (default: 200)\n\
         \x20   -h, --help        this help\n\
         \n\
         DB_PATH defaults to %USERPROFILE%\\.cc-switch\\cc-switch.db\n\
         \n\
         KEYS:\n\
         \x20   q/Esc quit · r refresh · Tab/1-5 page · t range · s sort\n\
         \x20   a/p/m agent/provider/model pickers · ↑↓ move · Enter/click drill-down\n\
         \x20   Trend: x metric · Requests: f status, n/b page\n\
         \n\
         MOUSE: left-click tabs/filters/rows · wheel to scroll selection\n"
    );
}

fn parse_range(v: &str) -> Range {
    match v.to_ascii_lowercase().as_str() {
        "all" => Range::All,
        "live" => Range::Live,
        "today" => Range::Today,
        "7" | "7d" => Range::Days(7),
        "30" | "30d" => Range::Days(30),
        other => match other.trim_end_matches('d').parse::<u32>() {
            Ok(n) => Range::Days(n),
            Err(_) => Range::All,
        },
    }
}

fn parse_page(v: &str) -> Page {
    match v.to_ascii_lowercase().as_str() {
        "trend" => Page::Trend,
        "requests" => Page::Requests,
        "providers" => Page::Providers,
        "models" => Page::Models,
        _ => Page::Overview,
    }
}

fn default_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cc-switch").join("cc-switch.db")
}

fn mtime_now(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

// ---------- headless dump ----------

fn dump(reader: &Reader, filter: &UsageFilter, by_model: bool) -> anyhow::Result<()> {
    let meta = reader.meta()?;
    let s = reader.summary(filter)?;
    println!("cc-switch usage · range: {} · READ-ONLY", filter.range.label());
    println!("db: {}", meta.db_path.display());
    if let Some(t) = meta.last_write {
        println!(
            "last write: {} · rows raw {} / rollups {}",
            local_hms_of(t),
            human_int(meta.raw_rows),
            human_int(meta.rollup_rows)
        );
    }
    println!(
        "\nSUMMARY  reqs {} · tokens {} · hit {} · cost {} · success {}",
        human_int(s.reqs),
        human_tokens(s.total_tokens()),
        pct(s.cache_hit()),
        human_cost(s.cost),
        pct(s.success_rate())
    );
    println!();

    if by_model {
        let models = reader.model_stats(filter)?;
        println!("{:<34}{:>8}{:>12}{:>12}{:>10}{:>12}{:>8}{:>12}", "Model", "Reqs", "Fresh In", "Cache Read", "Output", "Total Tok", "Hit %", "Cost");
        let sep = "-".repeat(110);
        println!("{sep}");
        for m in &models {
            println!(
                "{:<34}{:>8}{:>12}{:>12}{:>10}{:>12}{:>8}{:>12}",
                trunc(&m.model, 34),
                human_int(m.reqs),
                human_tokens(m.fresh),
                human_tokens(m.cached),
                human_tokens(m.output),
                human_tokens(m.total_tokens()),
                pct(m.cache_hit()),
                human_cost(m.cost)
            );
        }
    } else {
        let agents = reader.agent_stats(filter)?;
        println!("{:<16}{:>8}{:>12}{:>12}{:>10}{:>12}{:>8}{:>12}", "Agent", "Reqs", "Fresh In", "Cache Read", "Output", "Total Tok", "Hit %", "Cost");
        let sep = "-".repeat(92);
        println!("{sep}");
        for a in &agents {
            println!(
                "{:<16}{:>8}{:>12}{:>12}{:>10}{:>12}{:>8}{:>12}",
                trunc(&a.agent, 16),
                human_int(a.reqs),
                human_tokens(a.fresh),
                human_tokens(a.cached),
                human_tokens(a.output),
                human_tokens(a.total_tokens()),
                pct(a.cache_hit()),
                human_cost(a.cost)
            );
        }
    }
    Ok(())
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

// ---------- headless single-frame render ----------

fn render_frame(reader: &Reader, filter: &UsageFilter, page: Page) -> anyhow::Result<()> {
    let mut app = App::new();
    app.filter = filter.clone();
    app.page = page;
    app.refresh_options(reader);
    app.refresh_active(reader);

    let (w, h) = (120u16, 30u16);
    let mut term = Terminal::new(TestBackend::new(w, h))?;
    term.draw(|f| ui::draw(f, &mut app))?;
    let buf = term.backend().buffer().clone();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            match buf.cell((x, y)) {
                Some(c) => line.push_str(c.symbol()),
                None => line.push(' '),
            }
        }
        println!("{}", line.trim_end());
    }
    Ok(())
}

// ---------- TUI ----------

struct TermGuard;
impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;
    term.clear()?;
    Ok(term)
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

fn run_tui(reader: &Reader, path: &Path, filter: UsageFilter, interval_ms: u64) -> anyhow::Result<()> {
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        orig_hook(info);
    }));

    let mut app = App::new();
    app.filter = filter;
    app.refresh_options(reader);
    app.refresh_active(reader);
    app.last_mtime = mtime_now(path);

    let mut term = setup_terminal()?;
    let _guard = TermGuard;
    let poll = Duration::from_millis(interval_ms.max(50));

    while !app.quit {
        term.draw(|f| ui::draw(f, &mut app))?;

        if event::poll(poll)? {
            let ev = event::read()?;
            if let Some(action) = input::map_event(&app, ev) {
                app.apply(action, reader);
            }
        }

        // refresh on write (mtime change) or every 5s as a fallback
        let cur = mtime_now(path);
        let due = cur.is_some() && cur != app.last_mtime;
        let fallback = app
            .last_refresh
            .map(|t| t.elapsed().unwrap_or_default() >= Duration::from_secs(5))
            .unwrap_or(true);
        if due || fallback {
            app.refresh_active(reader);
            if cur.is_some() {
                app.last_mtime = cur;
            }
        }
    }
    Ok(())
}
