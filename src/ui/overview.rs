use super::common::{dim, human_cost, human_int, human_tokens, pct, ra, register_rows, truncate};
use crate::app::{App, HitRegion, UiAction};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

const W: [usize; 7] = [8, 10, 11, 9, 11, 7, 11];

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, hits: &mut Vec<HitRegion>) {
    let block = Block::default().borders(Borders::ALL).title(" Usage by agent ");
    let inner = block.inner(area);

    let hdr_style = dim().add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(Span::styled("Agent", hdr_style)),
        Cell::from(Span::styled(ra("Reqs", W[0]), hdr_style)),
        Cell::from(Span::styled(ra("Fresh In", W[1]), hdr_style)),
        Cell::from(Span::styled(ra("Cache Read", W[2]), hdr_style)),
        Cell::from(Span::styled(ra("Output", W[3]), hdr_style)),
        Cell::from(Span::styled(ra("Total Tok", W[4]), hdr_style)),
        Cell::from(Span::styled(ra("Hit %", W[5]), hdr_style)),
        Cell::from(Span::styled(ra("Cost", W[6]), hdr_style)),
    ]);

    let rows: Vec<Row> = app.agents.iter().map(|a| row(a.agent.clone(), a.reqs, a.fresh, a.cached, a.output, a.total_tokens(), a.cache_hit(), a.cost)).collect();
    let widths = widths();
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    let sel = app.overview.table.selected();
    let len = app.agents.len();
    frame.render_stateful_widget(table, area, &mut app.overview.table);
    register_rows(hits, inner, 1, len, sel, |i| UiAction::ActivateRow(i));
}

fn widths() -> Vec<Constraint> {
    vec![
        Constraint::Min(12),
        Constraint::Length(W[0] as u16),
        Constraint::Length(W[1] as u16),
        Constraint::Length(W[2] as u16),
        Constraint::Length(W[3] as u16),
        Constraint::Length(W[4] as u16),
        Constraint::Length(W[5] as u16),
        Constraint::Length(W[6] as u16),
    ]
}

pub fn row(label: String, reqs: i64, fresh: i64, cached: i64, output: i64, total: i64, hit: f64, cost: f64) -> Row<'static> {
    let hit_style = if hit >= 0.8 {
        Style::default().fg(Color::Green)
    } else if hit >= 0.4 {
        Style::default().fg(Color::Yellow)
    } else {
        dim()
    };
    let gray = Style::default().fg(Color::Gray);
    Row::new(vec![
        Cell::from(Span::styled(truncate(&label, 40), Style::default().fg(Color::White))),
        Cell::from(Span::styled(ra(&human_int(reqs), W[0]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(fresh), W[1]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(cached), W[2]), dim())),
        Cell::from(Span::styled(ra(&human_tokens(output), W[3]), gray)),
        Cell::from(Span::styled(ra(&human_tokens(total), W[4]), Style::default().fg(Color::White))),
        Cell::from(Span::styled(ra(&pct(hit), W[5]), hit_style)),
        Cell::from(Span::styled(ra(&human_cost(cost), W[6]), Style::default().fg(Color::Yellow))),
    ])
}
