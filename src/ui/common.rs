use crate::app::{App, HitRegion, Page, UiAction};
use chrono::{Local, TimeZone};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::time::{Duration, SystemTime};

// ---------- formatting ----------

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
    let b = s.as_bytes();
    let mut out = String::new();
    for (i, ch) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*ch as char);
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
        format!("{:.2}GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.2}MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1}KB", f / 1e3)
    } else {
        format!("{b}B")
    }
}

pub fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

pub fn human_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else {
        format!("{:.0}ms", ms)
    }
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

pub fn local_hms(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(d) => d.format("%H:%M:%S").to_string(),
        None => "--:--:--".into(),
    }
}

pub fn local_hms_of(t: SystemTime) -> String {
    let ts = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    local_hms(ts)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// right-align a string within `w` columns
pub fn ra(s: &str, w: usize) -> String {
    if s.chars().count() >= w {
        s.to_string()
    } else {
        format!("{:>w$}", s)
    }
}

/// Register per-row click targets for a table body, approximating ratatui's
/// scroll offset so clicks map to the right data index in the common case.
pub fn register_rows<F>(
    hits: &mut Vec<HitRegion>,
    inner: Rect,
    header_h: u16,
    len: usize,
    selected: Option<usize>,
    mk: F,
) where
    F: Fn(usize) -> UiAction,
{
    let visible = inner.height.saturating_sub(header_h) as usize;
    if visible == 0 || len == 0 {
        return;
    }
    let offset = match selected {
        Some(s) if s >= visible => s - visible + 1,
        _ => 0,
    };
    for d in 0..visible {
        let idx = offset + d;
        if idx >= len {
            break;
        }
        hits.push(HitRegion {
            rect: Rect { x: inner.x, y: inner.y + header_h + d as u16, width: inner.width, height: 1 },
            action: mk(idx),
        });
    }
}


// ---------- header ----------

pub fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.summary;
    let m = &app.meta;

    let summary = Line::from(vec![
        Span::styled("Requests ", dim()),
        Span::styled(human_int(s.reqs), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("   Tokens ", dim()),
        Span::styled(human_tokens(s.total_tokens()), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("   Hit ", dim()),
        Span::styled(pct(s.cache_hit()), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled("   Cost ", dim()),
        Span::styled(human_cost(s.cost), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("   Success ", dim()),
        Span::styled(pct(s.success_rate()), Style::default().fg(Color::Cyan)),
    ]);

    let last_write = match m.last_write {
        Some(t) => match m.last_write_elapsed() {
            Some(e) => format!("{} ({})", local_hms_of(t), rel_time(e)),
            None => local_hms_of(t),
        },
        None => "—".into(),
    };
    let meta = Line::from(vec![
        Span::styled("DB ", dim()),
        Span::raw(m.db_path.display().to_string()),
        Span::styled(format!("  ·  {}  ·  {}  ·  ", m.journal_mode.to_uppercase(), human_bytes(m.db_bytes)), dim()),
        Span::styled(" READ-ONLY ", Style::default().fg(Color::Black).bg(Color::Green)),
        Span::styled(format!("  ·  last write {last_write}  ·  refreshed {} ({}x)",
            app.last_refresh.map(local_hms_of).unwrap_or_else(|| "—".into()), app.refresh_count), dim()),
    ]);

    let status_style = if app.status_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    let status = Line::from(vec![
        Span::styled("status ", dim()),
        Span::styled(format!("[{}]", app.status), status_style),
        Span::styled("   ·   filters affect every page   ·   local-time ranges", dim()),
    ]);

    frame.render_widget(Paragraph::new(vec![summary, meta, status]), area);
}

// ---------- tabs ----------

pub fn draw_tabs(frame: &mut Frame, app: &App, area: Rect, hits: &mut Vec<HitRegion>) {
    let mut x = area.x;
    let mut spans = Vec::new();
    spans.push(Span::styled(" ", dim()));
    x += 1;
    for p in Page::ORDER {
        let active = p == app.page;
        let label = format!(" {} ", p.title());
        let w = label.chars().count() as u16;
        let style = if active {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label.clone(), style));
        spans.push(Span::styled(" ", dim()));
        hits.push(HitRegion {
            rect: Rect { x, y: area.y, width: w, height: 1 },
            action: UiAction::SetPage(p),
        });
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------- filter bar ----------

pub fn draw_filters(frame: &mut Frame, app: &App, area: Rect, hits: &mut Vec<HitRegion>) {
    let f = &app.filter;
    let agent_txt = f.agent.clone().unwrap_or_else(|| "All".into());
    let prov_txt = f.provider.as_ref().map(|p| p.display_name.clone()).unwrap_or_else(|| "All".into());
    let model_txt = f.model.clone().unwrap_or_else(|| "All".into());

    let chips: Vec<(String, Style, UiAction)> = vec![
        (format!("Range: [{}]", f.range.label()), Style::default().fg(Color::Magenta), UiAction::CycleRange),
        (format!("Agent: [{}]", agent_txt), Style::default().fg(Color::Blue), UiAction::OpenAgentPicker),
        (format!("Provider: [{}]", truncate(&prov_txt, 18)), Style::default().fg(Color::Blue), UiAction::OpenProviderPicker),
        (format!("Model: [{}]", truncate(&model_txt, 18)), Style::default().fg(Color::Blue), UiAction::OpenModelPicker),
    ];

    let mut x = area.x;
    let mut spans = Vec::new();
    for (label, style, action) in chips {
        let w = label.chars().count() as u16;
        spans.push(Span::styled(label, style));
        spans.push(Span::styled("  ", dim()));
        hits.push(HitRegion { rect: Rect { x, y: area.y, width: w, height: 1 }, action });
        x += w + 2;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------- footer ----------

pub fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let b = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(" q", b), Span::styled(" quit  ", dim()),
        Span::styled("r", b), Span::styled(" refresh  ", dim()),
        Span::styled("Tab/1-5", b), Span::styled(" page  ", dim()),
        Span::styled("t", b), Span::styled(" range  ", dim()),
        Span::styled("a/p/m", b), Span::styled(" filters  ", dim()),
        Span::styled("↑↓", b), Span::styled(" move  ", dim()),
    ];
    match app.page {
        Page::Overview | Page::Providers | Page::Models => {
            spans.push(Span::styled("s", b));
            spans.push(Span::styled(" sort  ", dim()));
            spans.push(Span::styled("Enter/click", b));
            spans.push(Span::styled(" drill-down  ", dim()));
        }
        Page::Trend => {
            spans.push(Span::styled("x", b));
            spans.push(Span::styled(" metric  ", dim()));
        }
        Page::Requests => {
            spans.push(Span::styled("f", b));
            spans.push(Span::styled(" status  ", dim()));
            spans.push(Span::styled("n/b", b));
            spans.push(Span::styled(" page  ", dim()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------- popup picker ----------

pub fn draw_popup(frame: &mut Frame, app: &App, area: Rect) {
    let items = app.popup_items();
    let title = match app.popup {
        crate::app::Popup::Agent => "Select agent",
        crate::app::Popup::Provider => "Select provider",
        crate::app::Popup::Model => "Select model",
        crate::app::Popup::None => return,
    };
    let width = items.iter().map(|s| s.chars().count()).max().unwrap_or(20).clamp(24, 60) as u16 + 4;
    let height = (items.len() as u16 + 2).min(area.height.saturating_sub(2)).max(3);
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|s| ListItem::new(Span::styled(s.clone(), Style::default().fg(Color::White))))
        .collect();
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title(format!(" {title} ")).title_alignment(Alignment::Left))
        .highlight_style(Style::default().bg(Color::Rgb(40, 40, 80)).fg(Color::Cyan))
        .highlight_symbol("▸ ");
    let mut state = ListState::default();
    state.select(Some(app.popup_sel.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(list, popup_area, &mut state);
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(width)) / 2),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(v[1]);
    h[1]
}
