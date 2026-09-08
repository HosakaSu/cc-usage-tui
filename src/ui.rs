use crate::app::{App, View};
use crate::db::Stat;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------- formatting helpers ----------

pub fn human_tokens(n: i64) -> String {
    let a = n.abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    if a >= 1e9 {
        format!("{sign}{:.2}B", a / 1e9)
    } else if a >= 1e6 {
        format!("{sign}{:.2}M", a / 1e6)
    } else if a >= 1e3 {
        format!("{sign}{:.1}K", a / 1e3)
    } else {
        format!("{sign}{:.0}", a)
    }
}

pub fn human_int(n: i64) -> String {
    let s = n.abs().to_string();
    let bytes = s.as_bytes();
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

pub fn human_cost(c: f64) -> String {
    if c == 0.0 {
        "$0".into()
    } else if c < 0.01 {
        format!("${c:.4}")
    } else if c < 1.0 {
        format!("${c:.3}")
    } else if c < 1000.0 {
        format!("${c:.2}")
    } else {
        format!("${}", human_int(c.round() as i64))
    }
}

pub fn human_bytes(b: u64) -> String {
    let f = b as f64;
    if f >= 1e9 {
        format!("{:.2} GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.2} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1} KB", f / 1e3)
    } else {
        format!("{b} B")
    }
}

pub fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

pub fn rel_time(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86400)
    }
}

/// UTC HH:MM:SS from a SystemTime.
pub fn utc_hms(t: SystemTime) -> String {
    let s = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = s % 86400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// right-align a string within `w` columns
fn ra(s: &str, w: usize) -> String {
    if s.chars().count() >= w {
        s.to_string()
    } else {
        format!("{:>w$}", s)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

// numeric column widths (must match the Length constraints in draw_table)
const W: [usize; 7] = [8, 10, 11, 9, 11, 7, 11];

// ---------- drawing ----------

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // header
            Constraint::Min(6),    // table
            Constraint::Length(1), // footer
        ])
        .split(area);

    draw_header(frame, app, chunks[0]);
    draw_table(frame, app, chunks[1]);
    draw_footer(frame, chunks[2]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let m = &app.meta;
    let last_write = match m.last_write {
        Some(t) => match m.last_write_elapsed() {
            Some(e) => format!("{} UTC ({})", utc_hms(t), rel_time(e)),
            None => format!("{} UTC", utc_hms(t)),
        },
        None => "—".into(),
    };
    let refreshed = match app.last_refresh {
        Some(t) => format!("{} UTC ({}x)", utc_hms(t), app.refresh_count),
        None => "—".into(),
    };
    let journal = m.journal_mode.to_uppercase();
    let ro_tag = Span::styled(" READ-ONLY ", Style::default().fg(Color::Black).bg(Color::Green));
    let dim = Style::default().fg(Color::DarkGray);

    let l1 = Line::from(vec![
        Span::styled("DB ", dim),
        Span::raw(app.db_path.display().to_string()),
    ]);
    let l2 = Line::from(vec![
        Span::styled("journal ", dim),
        Span::styled(journal, Style::default().fg(Color::Yellow)),
        Span::styled(format!("  ·  {}  ·  ", human_bytes(m.db_bytes)), dim),
        ro_tag,
        Span::styled(format!("  ·  rows raw {} / rollups {}", human_int(m.raw_rows), human_int(m.rollup_rows)), dim),
    ]);
    let l3 = Line::from(vec![
        Span::styled("last write ", dim),
        Span::styled(last_write, Style::default().fg(Color::Cyan)),
        Span::styled(format!("  ·  refreshed {refreshed}"), dim),
    ]);
    let status_style = if app.status_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    let l4 = Line::from(vec![
        Span::styled("range ", dim),
        Span::styled(app.range.label(), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("  ·  view ", dim),
        Span::styled(app.view.label(), Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
        Span::styled("  ·  sort ", dim),
        Span::styled(app.sort.label(), Style::default().fg(Color::Blue)),
        Span::styled("  ·  ", dim),
        Span::styled(format!("[{}]", app.status), status_style),
    ]);
    let para = Paragraph::new(vec![l1, l2, l3, l4]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" cc-switch usage ")
            .title_alignment(Alignment::Left),
    );
    frame.render_widget(para, area);
}

fn stat_row(s: &Stat, name_w: usize, style: Style) -> Row<'_> {
    let name = truncate(&s.label(), name_w);
    let hit = s.cache_hit();
    let hit_style = if hit >= 0.8 {
        Style::default().fg(Color::Green)
    } else if hit >= 0.4 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let gray = Style::default().fg(Color::Gray);
    let cells = vec![
        Cell::from(Span::styled(name, style)),
        Cell::from(Span::styled(ra(&human_int(s.reqs), W[0]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(s.fresh), W[1]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(s.cached), W[2]), Style::default().fg(Color::DarkGray))),
        Cell::from(Span::styled(ra(&human_tokens(s.output), W[3]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(s.total_tokens()), W[4]), Style::default().fg(Color::White))),
        Cell::from(Span::styled(ra(&pct(hit), W[5]), hit_style)),
        Cell::from(Span::styled(ra(&human_cost(s.cost), W[6]), Style::default().fg(Color::Yellow))),
    ];
    Row::new(cells)
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let hdr_style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(Span::styled("Agent", hdr_style)),
        Cell::from(Span::styled(ra("Reqs", W[0]), hdr_style)),
        Cell::from(Span::styled(ra("Fresh In", W[1]), hdr_style)),
        Cell::from(Span::styled(ra("Cache Read", W[2]), hdr_style)),
        Cell::from(Span::styled(ra("Output", W[3]), hdr_style)),
        Cell::from(Span::styled(ra("Total Tok", W[4]), hdr_style)),
        Cell::from(Span::styled(ra("Hit %", W[5]), hdr_style)),
        Cell::from(Span::styled(ra("Cost", W[6]), hdr_style)),
    ])
    .style(hdr_style);
    let (first_min, name_w) = match app.view {
        View::Agents => (12usize, 22usize),
        View::Models => (26usize, 34usize),
    };
    let widths = vec![
        Constraint::Min(first_min as u16),
        Constraint::Length(W[0] as u16),
        Constraint::Length(W[1] as u16),
        Constraint::Length(W[2] as u16),
        Constraint::Length(W[3] as u16),
        Constraint::Length(W[4] as u16),
        Constraint::Length(W[5] as u16),
        Constraint::Length(W[6] as u16),
    ];

    let mut rows: Vec<Row> = Vec::with_capacity(app.stats.len() + 1);
    for s in &app.stats {
        rows.push(stat_row(s, name_w, Style::default().fg(Color::White)));
    }
    let total_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    rows.push(stat_row(&app.total, name_w, total_style).style(total_style));

    let title = if app.stats.is_empty() {
        format!(" no data for range: {} ", app.range.label())
    } else {
        match app.view {
            View::Agents => " Usage by agent ".to_string(),
            View::Models => " Usage by model ".to_string(),
        }
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    let mut state = TableState::default();
    if !app.stats.is_empty() {
        state.select(Some(app.selected.min(app.stats.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let b = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let keys = Line::from(vec![
        Span::styled(" q", b),
        Span::styled(" quit   ", dim),
        Span::styled("r", b),
        Span::styled(" refresh   ", dim),
        Span::styled("t", b),
        Span::styled(" range   ", dim),
        Span::styled("v", b),
        Span::styled(" view   ", dim),
        Span::styled("s", b),
        Span::styled(" sort   ", dim),
        Span::styled("↑↓/jk", b),
        Span::styled(" select   ", dim),
        Span::styled("(auto-refreshes on each cc-switch write)", dim),
    ]);
    frame.render_widget(Paragraph::new(keys), area);
}
