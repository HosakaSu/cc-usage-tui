pub mod common;
pub mod overview;
pub mod requests;
pub mod stats;
pub mod trend;

use crate::app::{App, Page, Popup};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let mut hits = std::mem::take(&mut app.hit_regions);
    hits.clear();

    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header (summary/meta/status)
            Constraint::Length(1), // page tabs
            Constraint::Length(1), // filter bar
            Constraint::Min(6),    // page body
            Constraint::Length(1), // footer
        ])
        .split(area);

    common::draw_header(frame, app, chunks[0]);
    common::draw_tabs(frame, app, chunks[1], &mut hits);
    common::draw_filters(frame, app, chunks[2], &mut hits);

    match app.page {
        Page::Overview => overview::draw(frame, app, chunks[3], &mut hits),
        Page::Trend => trend::draw(frame, app, chunks[3]),
        Page::Requests => requests::draw(frame, app, chunks[3], &mut hits),
        Page::Providers => stats::draw_providers(frame, app, chunks[3], &mut hits),
        Page::Models => stats::draw_models(frame, app, chunks[3], &mut hits),
    }

    common::draw_footer(frame, app, chunks[4]);

    if app.popup != Popup::None {
        common::draw_popup(frame, app, area);
    }

    app.hit_regions = hits;
}
