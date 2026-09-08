mod app;
mod db;
mod ui;

use app::{sort_stats, App, SortKey, View};
use db::{Range, Reader};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
    let mut interval_ms: u64 = 200;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
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
            "--interval" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    interval_ms = v.parse().unwrap_or(200);
                }
            }
            other => {
                if other.starts_with("--range=") {
                    range = parse_range(&other["--range=".len()..]);
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

    let result = if want_dump {
        dump(&reader, range, by_model)
    } else if frame_mode {
        render_frame(&reader, range, by_model)
    } else {
        run_tui(&reader, &path, range, interval_ms)
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "cc-usage-tui — read-only realtime usage dashboard for cc-switch\n\
         \n\
         USAGE:\n    cc-usage-tui [DB_PATH] [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \x20   --dump            print a one-shot text report and exit (no TUI)\n\
         \x20   --frame           render one TUI frame as text and exit (no tty needed)\n\
         \x20   --by-model        break down by model instead of agent (with --dump/--frame)\n\
         \x20   --range <R>       all | live | today | 7 | 30   (default: all)\n\
         \x20   --interval <ms>   poll interval for write detection (default: 200)\n\
         \x20   -h, --help        this help\n\
         \n\
         DB_PATH defaults to %USERPROFILE%\\.cc-switch\\cc-switch.db\n\
         \n\
         KEYBINDINGS (TUI):\n\
         \x20   q / Esc   quit          r   refresh now\n\
         \x20   t   cycle range         v   toggle agents/models\n\
         \x20   s   cycle sort          ↑↓ / j k   move selection\n"
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

fn dump(reader: &Reader, range: Range, by_model: bool) -> anyhow::Result<()> {
    let meta = reader.meta()?;
    let mut stats = reader.stats(range, by_model)?;
    sort_stats(&mut stats, SortKey::Cost);
    let total = db::total_of(&stats);

    let name_w = if by_model { 30 } else { 14 };

    println!("cc-switch usage  ·  range: {}  ·  READ-ONLY", range.label());
    println!("db: {}", meta.db_path.display());
    if let Some(t) = meta.last_write {
        println!(
            "last write: {} UTC  ·  rows: raw {} / rollups {}",
            ui::utc_hms(t),
            ui::human_int(meta.raw_rows),
            ui::human_int(meta.rollup_rows)
        );
    }
    println!();
    println!(
        "{:<nw$}  {:>8}  {:>10}  {:>11}  {:>9}  {:>11}  {:>7}  {:>11}",
        if by_model { "Agent · Model" } else { "Agent" },
        "Reqs",
        "Fresh In",
        "Cache Read",
        "Output",
        "Total Tok",
        "Hit %",
        "Cost",
        nw = name_w
    );
    let sep = "-".repeat(name_w + 8 + 10 + 11 + 9 + 11 + 7 + 11 + 14);
    println!("{sep}");
    for s in &stats {
        println!(
            "{:<nw$}  {:>8}  {:>10}  {:>11}  {:>9}  {:>11}  {:>7}  {:>11}",
            trunc(&s.label(), name_w),
            ui::human_int(s.reqs),
            ui::human_tokens(s.fresh),
            ui::human_tokens(s.cached),
            ui::human_tokens(s.output),
            ui::human_tokens(s.total_tokens()),
            ui::pct(s.cache_hit()),
            ui::human_cost(s.cost),
            nw = name_w
        );
    }
    println!("{sep}");
    println!(
        "{:<nw$}  {:>8}  {:>10}  {:>11}  {:>9}  {:>11}  {:>7}  {:>11}",
        "TOTAL",
        ui::human_int(total.reqs),
        ui::human_tokens(total.fresh),
        ui::human_tokens(total.cached),
        ui::human_tokens(total.output),
        ui::human_tokens(total.total_tokens()),
        ui::pct(total.cache_hit()),
        ui::human_cost(total.cost),
        nw = name_w
    );
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

// ---------- headless single-frame render (text screenshot of the TUI) ----------

fn render_frame(reader: &Reader, range: Range, by_model: bool) -> anyhow::Result<()> {
    let mut app = App::new(reader.path().to_path_buf());
    app.range = range;
    if by_model {
        app.view = View::Models;
    }
    app.refresh(reader);

    let (w, h) = (116u16, 26u16);
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
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;
    term.clear()?;
    Ok(term)
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn run_tui(reader: &Reader, path: &Path, range: Range, interval_ms: u64) -> anyhow::Result<()> {
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        orig_hook(info);
    }));

    let mut app = App::new(path.to_path_buf());
    app.range = range;
    app.refresh(reader);
    app.last_mtime = mtime_now(path);

    let mut term = setup_terminal()?;
    let _guard = TermGuard;
    let poll = Duration::from_millis(interval_ms.max(50));

    while !app.quit {
        term.draw(|f| ui::draw(f, &mut app))?;

        if event::poll(poll)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    handle_key(&mut app, reader, k);
                }
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
            app.refresh(reader);
            if cur.is_some() {
                app.last_mtime = cur;
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, reader: &Reader, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Char('c') if ctrl => app.quit = true,
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('r') => app.refresh(reader),
        KeyCode::Char('t') => {
            app.cycle_range();
            app.refresh(reader);
        }
        KeyCode::Char('v') => {
            app.view = app.view.toggle();
            app.selected = 0;
            app.refresh(reader);
        }
        KeyCode::Char('s') => app.cycle_sort(),
        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
        KeyCode::Down | KeyCode::Char('j') => app.move_down(),
        KeyCode::Home => app.selected = 0,
        KeyCode::End => app.selected = app.stats.len().saturating_sub(1),
        _ => {}
    }
}
