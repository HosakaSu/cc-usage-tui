use super::common::{dim, human_cost, human_int, human_tokens};
use crate::app::{App, TrendMetric};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let pts = &app.trend;
    let title = format!(
        " Trend · {} · metric: {} (x = {}) ",
        app.filter.range.label(),
        app.trend_state.metric.label(),
        if app.filter.range.hourly() { "hour" } else { "day" }
    );
    let block = Block::default().borders(Borders::ALL).title(title);

    if pts.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled("no data in range", dim())).block(block), area);
        return;
    }

    let n = pts.len();
    let x_max = (n - 1).max(1) as f64;

    let series: Vec<(&str, Color, Vec<(f64, f64)>)> = match app.trend_state.metric {
        TrendMetric::Tokens => vec![
            ("Fresh", Color::Cyan, pts.iter().enumerate().map(|(i, p)| (i as f64, p.fresh as f64)).collect()),
            ("Cache read", Color::Gray, pts.iter().enumerate().map(|(i, p)| (i as f64, p.cached as f64)).collect()),
            ("Output", Color::Green, pts.iter().enumerate().map(|(i, p)| (i as f64, p.output as f64)).collect()),
        ],
        TrendMetric::Cost => vec![("Cost", Color::Yellow, pts.iter().enumerate().map(|(i, p)| (i as f64, p.cost)).collect())],
        TrendMetric::Requests => vec![("Requests", Color::Magenta, pts.iter().enumerate().map(|(i, p)| (i as f64, p.reqs as f64)).collect())],
    };

    let fy = |v: f64| match app.trend_state.metric {
        TrendMetric::Tokens => human_tokens(v as i64),
        TrendMetric::Cost => human_cost(v),
        TrendMetric::Requests => human_int(v as i64),
    };

    let y_max = series
        .iter()
        .flat_map(|s| s.2.iter().map(|p| p.1))
        .fold(0.0f64, f64::max)
        .max(1.0);

    let datasets: Vec<Dataset> = series
        .iter()
        .map(|(name, color, data)| {
            Dataset::default()
                .name(*name)
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data)
        })
        .collect();

    let x_labels = vec![
        Span::styled(pts[0].label.clone(), dim()),
        Span::styled(pts[n / 2].label.clone(), dim()),
        Span::styled(pts[n - 1].label.clone(), dim()),
    ];
    let y_labels = vec![
        Span::styled(fy(0.0), dim()),
        Span::styled(fy(y_max / 2.0), dim()),
        Span::styled(fy(y_max), dim()),
    ];

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds([0.0, x_max]).labels(x_labels))
        .y_axis(Axis::default().bounds([0.0, y_max]).labels(y_labels));

    frame.render_widget(chart, area);
}
