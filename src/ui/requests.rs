use super::common::{dim, human_cost, human_ms, human_tokens, local_hms, ra, register_rows, truncate};
use crate::app::{App, HitRegion, UiAction};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

const W: [usize; 5] = [9, 9, 8, 8, 6]; // fresh, cache, out, cost, status

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, hits: &mut Vec<HitRegion>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(8)])
        .split(area);

    draw_table(frame, app, chunks[0], hits);
    draw_detail(frame, app, chunks[1]);
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect, hits: &mut Vec<HitRegion>) {
    let total = app.requests.total;
    let page = app.request_state.page;
    let pages = if total == 0 { 0 } else { (total - 1) / app.page_size.max(1) };
    let status_txt = match app.request_state.status {
        Some(s) => format!(" · status={s}"),
        None => String::new(),
    };
    let title = format!(" Requests · page {}/{} · {} total{} ", page + 1, pages + 1, total, status_txt);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);

    let hdr = dim().add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(Span::styled("Time", hdr)),
        Cell::from(Span::styled("Agent", hdr)),
        Cell::from(Span::styled("Provider", hdr)),
        Cell::from(Span::styled("Model", hdr)),
        Cell::from(Span::styled(ra("Fresh", W[0]), hdr)),
        Cell::from(Span::styled(ra("Cache", W[1]), hdr)),
        Cell::from(Span::styled(ra("Out", W[2]), hdr)),
        Cell::from(Span::styled(ra("Cost", W[3]), hdr)),
        Cell::from(Span::styled(ra("Code", W[4]), hdr)),
    ]);

    let rows: Vec<Row> = app
        .requests
        .rows
        .iter()
        .map(|r| {
            let gray = Style::default().fg(Color::Gray);
            let ok = (200..300).contains(&r.status_code);
            let code_style = if ok {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            Row::new(vec![
                Cell::from(Span::styled(local_hms(r.created_at), dim())),
                Cell::from(Span::styled(truncate(&r.app_type, 12), gray)),
                Cell::from(Span::styled(truncate(&r.provider_name, 20), Style::default().fg(Color::White))),
                Cell::from(Span::styled(truncate(&r.model, 22), gray)),
                Cell::from(Span::styled(ra(&human_tokens(r.fresh), W[0]), gray)),
                Cell::from(Span::styled(ra(&human_tokens(r.cached), W[1]), dim())),
                Cell::from(Span::styled(ra(&human_tokens(r.output), W[2]), gray)),
                Cell::from(Span::styled(ra(&human_cost(r.cost), W[3]), Style::default().fg(Color::Yellow))),
                Cell::from(Span::styled(ra(&r.status_code.to_string(), W[4]), code_style)),
            ])
        })
        .collect();

    let widths = vec![
        Constraint::Length(9),
        Constraint::Length(12),
        Constraint::Min(16),
        Constraint::Min(18),
        Constraint::Length(W[0] as u16),
        Constraint::Length(W[1] as u16),
        Constraint::Length(W[2] as u16),
        Constraint::Length(W[3] as u16),
        Constraint::Length(W[4] as u16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");
    let sel = app.request_state.table.selected();
    let len = app.requests.rows.len();
    frame.render_stateful_widget(table, area, &mut app.request_state.table);
    register_rows(hits, inner, 1, len, sel, |i| UiAction::ActivateRow(i));
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let sel = app.request_state.table.selected().unwrap_or(0);
    let Some(r) = app.requests.rows.get(sel) else {
        frame.render_widget(Paragraph::new(Span::styled("select a request…", dim())).block(block), area);
        return;
    };
    let label = |s: &str| Span::styled(format!("{:<14}", s), dim());
    let val = |s: String| Span::styled(s, Style::default().fg(Color::White));

    let pricing = r.pricing_model.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into());
    let req_model = r.request_model.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into());
    let first = r.first_token_ms.map(|m| human_ms(m as f64)).unwrap_or_else(|| "—".into());
    let dur = r.duration_ms.map(|m| human_ms(m as f64)).unwrap_or_else(|| "—".into());
    let err = r.error_message.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into());

    let lines = vec![
        Line::from(vec![label("request id"), val(r.request_id.clone())]),
        Line::from(vec![label("model"), val(format!("{}   (request: {} · pricing: {})", r.model, req_model, pricing))]),
        Line::from(vec![label("provider"), val(format!("{} · {}   (id: {})", r.provider_name, r.app_type, r.provider_id))]),
        Line::from(vec![label("source/status"), val(format!("{} · {}", r.data_source, r.status_code))]),
        Line::from(vec![label("latency"), val(format!("{} total · first token {} · duration {}", human_ms(r.latency_ms as f64), first, dur))]),
        Line::from(vec![
            label("tokens"),
            val(format!(
                "fresh {} · cache {} · create {} · out {} · cost {}",
                human_tokens(r.fresh),
                human_tokens(r.cached),
                human_tokens(r.ccreate),
                human_tokens(r.output),
                human_cost(r.cost)
            )),
        ]),
        Line::from(vec![label("error"), val(err)]),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
