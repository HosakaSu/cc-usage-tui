use crate::app::{App, Page, Popup, UiAction};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

/// Translate a terminal event into a semantic UiAction (or None to ignore).
pub fn map_event(app: &App, event: Event) -> Option<UiAction> {
    match event {
        Event::Key(k) if k.kind == KeyEventKind::Press => map_key(app, k),
        Event::Mouse(m) => map_mouse(app, m),
        _ => None,
    }
}

fn popup_open(app: &App) -> bool {
    app.popup != Popup::None
}

fn map_key(app: &App, k: KeyEvent) -> Option<UiAction> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);

    // popup modal captures navigation keys
    if popup_open(app) {
        return match k.code {
            KeyCode::Esc => Some(UiAction::ClosePopup),
            KeyCode::Enter => Some(UiAction::PopupConfirm),
            KeyCode::Up | KeyCode::Char('k') => Some(UiAction::PopupPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(UiAction::PopupNext),
            KeyCode::Char('c') if ctrl => Some(UiAction::Quit),
            _ => None,
        };
    }

    if ctrl {
        return match k.code {
            KeyCode::Char('c') => Some(UiAction::Quit),
            _ => None,
        };
    }

    match k.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(UiAction::Quit),
        KeyCode::Char('r') => Some(UiAction::Refresh),

        KeyCode::Tab | KeyCode::Right => Some(UiAction::NextPage),
        KeyCode::BackTab | KeyCode::Left => Some(UiAction::PrevPage),

        KeyCode::Char('1') => Some(UiAction::SetPage(Page::Overview)),
        KeyCode::Char('2') => Some(UiAction::SetPage(Page::Trend)),
        KeyCode::Char('3') => Some(UiAction::SetPage(Page::Requests)),
        KeyCode::Char('4') => Some(UiAction::SetPage(Page::Providers)),
        KeyCode::Char('5') => Some(UiAction::SetPage(Page::Models)),

        KeyCode::Char('t') => Some(UiAction::CycleRange),
        KeyCode::Char('s') => Some(UiAction::CycleSort),

        KeyCode::Char('a') => Some(UiAction::OpenAgentPicker),
        KeyCode::Char('p') => Some(UiAction::OpenProviderPicker),
        KeyCode::Char('m') => Some(UiAction::OpenModelPicker),

        KeyCode::Up | KeyCode::Char('k') => Some(UiAction::SelectPrev),
        KeyCode::Down | KeyCode::Char('j') => Some(UiAction::SelectNext),
        KeyCode::PageUp => Some(UiAction::PageUp),
        KeyCode::PageDown => Some(UiAction::PageDown),
        KeyCode::Home => Some(UiAction::SelectFirst),
        KeyCode::End => Some(UiAction::SelectLast),

        KeyCode::Enter => Some(row_enter(app)),

        // page-specific
        KeyCode::Char('x') if app.page == Page::Trend => Some(UiAction::CycleTrendMetric),
        KeyCode::Char('f') if app.page == Page::Requests => Some(UiAction::CycleRequestStatus),
        KeyCode::Char('n') if app.page == Page::Requests => Some(UiAction::RequestNextPage),
        KeyCode::Char('b') if app.page == Page::Requests => Some(UiAction::RequestPrevPage),

        _ => None,
    }
}

fn row_enter(app: &App) -> UiAction {
    match app.page {
        Page::Trend => UiAction::CycleTrendMetric,
        Page::Requests => UiAction::CycleRequestStatus,
        other => {
            // activate the selected row (drill-down filter)
            let sel = match other {
                Page::Overview => app.overview.table.selected(),
                Page::Providers => app.provider_state.table.selected(),
                Page::Models => app.model_state.table.selected(),
                _ => None,
            };
            match sel {
                Some(i) => UiAction::ActivateRow(i),
                None => UiAction::Noop,
            }
        }
    }
}

fn map_mouse(app: &App, m: MouseEvent) -> Option<UiAction> {
    if popup_open(app) {
        return match m.kind {
            MouseEventKind::ScrollUp => Some(UiAction::PopupPrev),
            MouseEventKind::ScrollDown => Some(UiAction::PopupNext),
            MouseEventKind::Down(MouseButton::Left) => Some(UiAction::ClosePopup),
            _ => None,
        };
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => app
            .hit_regions
            .iter()
            .rev()
            .find(|h| contains(h.rect, m.column, m.row))
            .map(|h| h.action.clone()),
        MouseEventKind::ScrollUp => Some(UiAction::SelectPrev),
        MouseEventKind::ScrollDown => Some(UiAction::SelectNext),
        _ => None,
    }
}

fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}
