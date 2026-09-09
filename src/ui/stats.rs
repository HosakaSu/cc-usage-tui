use super::common::{dim, human_cost, human_int, human_ms, human_tokens, pct, ra, register_rows, truncate};
use crate::app::{App, HitRegion, UiAction};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

// ---------------- Providers ----------------

const PW: [usize; 5] = [8, 11, 11, 8, 9]; // reqs, tokens, cost, success, lat

pub fn draw_providers(frame: &mut Frame, app: &mut App, area: Rect, hits: &mut Vec<HitRegion>) {
    let block = Block::default().borders(Borders::ALL).title(" Usage by provider ");
    let inner = block.inner(area);
    let hdr = dim().add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(Span::styled("Provider", hdr)),
        Cell::from(Span::styled("Agent", hdr)),
        Cell::from(Span::styled(ra("Reqs", PW[0]), hdr)),
        Cell::from(Span::styled(ra("Tokens", PW[1]), hdr)),
        Cell::from(Span::styled(ra("Cost", PW[2]), hdr)),
        Cell::from(Span::styled(ra("Success", PW[3]), hdr)),
        Cell::from(Span::styled(ra("Avg Lat", PW[4]), hdr)),
    ]);
    let rows: Vec<Row> = app
        .providers
        .iter()
        .map(|p| {
            let gray = Style::default().fg(Color::Gray);
            Row::new(vec![
                Cell::from(Span::styled(truncate(&p.provider_name, 26), Style::default().fg(Color::White))),
                Cell::from(Span::styled(truncate(&p.app_type, 12), dim())),
                Cell::from(Span::styled(ra(&human_int(p.reqs), PW[0]), gray)),
                Cell::from(Span::styled(ra(&human_tokens(p.total_tokens), PW[1]), gray)),
                Cell::from(Span::styled(ra(&human_cost(p.cost), PW[2]), Style::default().fg(Color::Yellow))),
                Cell::from(Span::styled(ra(&pct(p.success_rate()), PW[3]), Style::default().fg(Color::Cyan))),
                Cell::from(Span::styled(ra(&human_ms(p.avg_latency_ms), PW[4]), gray)),
            ])
        })
        .collect();
    let widths = vec![
        Constraint::Min(18),
        Constraint::Length(12),
        Constraint::Length(PW[0] as u16),
        Constraint::Length(PW[1] as u16),
        Constraint::Length(PW[2] as u16),
        Constraint::Length(PW[3] as u16),
        Constraint::Length(PW[4] as u16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");
    let sel = app.provider_state.table.selected();
    let len = app.providers.len();
    frame.render_stateful_widget(table, area, &mut app.provider_state.table);
    register_rows(hits, inner, 1, len, sel, |i| UiAction::ActivateRow(i));
}

// ---------------- Models ----------------

const MW: [usize; 8] = [8, 10, 11, 9, 11, 7, 9, 11];

pub fn draw_models(frame: &mut Frame, app: &mut App, area: Rect, hits: &mut Vec<HitRegion>) {
    let block = Block::default().borders(Borders::ALL).title(" Usage by model (effective pricing model) ");
    let inner = block.inner(area);
    let hdr = dim().add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(Span::styled("Model", hdr)),
        Cell::from(Span::styled(ra("Reqs", MW[0]), hdr)),
        Cell::from(Span::styled(ra("Fresh In", MW[1]), hdr)),
        Cell::from(Span::styled(ra("Cache Read", MW[2]), hdr)),
        Cell::from(Span::styled(ra("Output", MW[3]), hdr)),
        Cell::from(Span::styled(ra("Total Tok", MW[4]), hdr)),
        Cell::from(Span::styled(ra("Hit %", MW[5]), hdr)),
        Cell::from(Span::styled(ra("Avg/Req", MW[6]), hdr)),
        Cell::from(Span::styled(ra("Cost", MW[7]), hdr)),
    ]);
    let rows: Vec<Row> = app
        .models
        .iter()
        .map(|m| {
            let gray = Style::default().fg(Color::Gray);
            let hit = m.cache_hit();
            let hit_style = if hit >= 0.8 {
                Style::default().fg(Color::Green)
            } else if hit >= 0.4 {
                Style::default().fg(Color::Yellow)
            } else {
                dim()
            };
            Row::new(vec![
                Cell::from(Span::styled(truncate(&m.model, 34), Style::default().fg(Color::White))),
                Cell::from(Span::styled(ra(&human_int(m.reqs), MW[0]), gray)),
                Cell::from(Span::styled(ra(&human_tokens(m.fresh), MW[1]), gray)),
                Cell::from(Span::styled(ra(&human_tokens(m.cached), MW[2]), dim())),
                Cell::from(Span::styled(ra(&human_tokens(m.output), MW[3]), gray)),
                Cell::from(Span::styled(ra(&human_tokens(m.total_tokens()), MW[4]), Style::default().fg(Color::White))),
                Cell::from(Span::styled(ra(&pct(hit), MW[5]), hit_style)),
                Cell::from(Span::styled(ra(&human_tokens(m.avg_tokens_per_req() as i64), MW[6]), gray)),
                Cell::from(Span::styled(ra(&human_cost(m.cost), MW[7]), Style::default().fg(Color::Yellow))),
            ])
        })
        .collect();
    let widths = vec![
        Constraint::Min(22),
        Constraint::Length(MW[0] as u16),
        Constraint::Length(MW[1] as u16),
        Constraint::Length(MW[2] as u16),
        Constraint::Length(MW[3] as u16),
        Constraint::Length(MW[4] as u16),
        Constraint::Length(MW[5] as u16),
        Constraint::Length(MW[6] as u16),
        Constraint::Length(MW[7] as u16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");
    let sel = app.model_state.table.selected();
    let len = app.models.len();
    frame.render_stateful_widget(table, area, &mut app.model_state.table);
    register_rows(hits, inner, 1, len, sel, |i| UiAction::ActivateRow(i));
}
