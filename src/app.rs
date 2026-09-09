use crate::db::{
    AgentStat, FilterOptions, Meta, ModelStat, ProviderKey, ProviderStat, Range, Reader,
    RequestPage, TrendPoint, UsageFilter, UsageSummary,
};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Trend,
    Requests,
    Providers,
    Models,
}

impl Page {
    pub const ORDER: [Page; 5] = [Page::Overview, Page::Trend, Page::Requests, Page::Providers, Page::Models];
    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Trend => "Trend",
            Page::Requests => "Requests",
            Page::Providers => "Providers",
            Page::Models => "Models",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Cost,
    Tokens,
    Reqs,
    CacheHit,
    Name,
}

impl SortKey {
    pub const ORDER: [SortKey; 5] = [SortKey::Cost, SortKey::Tokens, SortKey::Reqs, SortKey::CacheHit, SortKey::Name];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cost => "Cost",
            SortKey::Tokens => "Tokens",
            SortKey::Reqs => "Reqs",
            SortKey::CacheHit => "Hit%",
            SortKey::Name => "Name",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrendMetric {
    #[default]
    Tokens,
    Cost,
    Requests,
}

impl TrendMetric {
    pub const ORDER: [TrendMetric; 3] = [TrendMetric::Tokens, TrendMetric::Cost, TrendMetric::Requests];
    pub fn label(self) -> &'static str {
        match self {
            TrendMetric::Tokens => "Tokens",
            TrendMetric::Cost => "Cost",
            TrendMetric::Requests => "Requests",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Popup {
    #[default]
    None,
    Agent,
    Provider,
    Model,
}

#[derive(Clone)]
pub enum UiAction {
    Quit,
    Refresh,
    SetPage(Page),
    NextPage,
    PrevPage,
    SetRange(Range),
    CycleRange,
    SetAgent(Option<String>),
    SetProvider(Option<ProviderKey>),
    SetModel(Option<String>),
    SelectNext,
    SelectPrev,
    PageUp,
    PageDown,
    SelectFirst,
    SelectLast,
    ActivateRow(usize),
    CycleTrendMetric,
    RequestNextPage,
    RequestPrevPage,
    CycleRequestStatus,
    CycleSort,
    OpenAgentPicker,
    OpenProviderPicker,
    OpenModelPicker,
    ClosePopup,
    PopupNext,
    PopupPrev,
    PopupConfirm,
    Noop,
}

#[derive(Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: UiAction,
}

#[derive(Default)]
pub struct OverviewState {
    pub table: TableState,
    pub sort: SortKey,
}
#[derive(Default)]
pub struct TrendState {
    pub metric: TrendMetric,
}
#[derive(Default)]
pub struct RequestState {
    pub table: TableState,
    pub page: u32,
    pub status: Option<i64>,
}
#[derive(Default)]
pub struct ProviderState {
    pub table: TableState,
    pub sort: SortKey,
}
#[derive(Default)]
pub struct ModelState {
    pub table: TableState,
    pub sort: SortKey,
}

pub struct App {
    pub page: Page,
    pub filter: UsageFilter,

    pub summary: UsageSummary,
    pub agents: Vec<AgentStat>,
    pub trend: Vec<TrendPoint>,
    pub requests: RequestPage,
    pub providers: Vec<ProviderStat>,
    pub models: Vec<ModelStat>,
    pub options: FilterOptions,

    pub overview: OverviewState,
    pub trend_state: TrendState,
    pub request_state: RequestState,
    pub provider_state: ProviderState,
    pub model_state: ModelState,

    pub popup: Popup,
    pub popup_sel: usize,

    pub hit_regions: Vec<HitRegion>,

    pub meta: Meta,
    pub status: String,
    pub status_error: bool,
    pub quit: bool,
    pub last_refresh: Option<SystemTime>,
    pub last_mtime: Option<SystemTime>,
    pub refresh_count: u64,
    pub page_size: u32,
}

impl Default for App {
    fn default() -> Self {
        Self {
            page: Page::Overview,
            filter: UsageFilter::default(),
            summary: UsageSummary::default(),
            agents: Vec::new(),
            trend: Vec::new(),
            requests: RequestPage::default(),
            providers: Vec::new(),
            models: Vec::new(),
            options: FilterOptions::default(),
            overview: OverviewState::default(),
            trend_state: TrendState::default(),
            request_state: RequestState::default(),
            provider_state: ProviderState::default(),
            model_state: ModelState::default(),
            popup: Popup::None,
            popup_sel: 0,
            hit_regions: Vec::new(),
            meta: Meta::default(),
            status: "loading…".into(),
            status_error: false,
            quit: false,
            last_refresh: None,
            last_mtime: None,
            refresh_count: 0,
            page_size: 20,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- data refresh ----

    /// Page-aware refresh: always meta+summary, plus only the active page's data.
    pub fn refresh_active(&mut self, reader: &Reader) {
        match self.query_active(reader) {
            Ok(()) => {
                self.refresh_count += 1;
                self.last_refresh = Some(SystemTime::now());
                self.status = "ok".into();
                self.status_error = false;
            }
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    fn query_active(&mut self, reader: &Reader) -> anyhow::Result<()> {
        self.meta = reader.meta()?;
        self.summary = reader.summary(&self.filter)?;
        match self.page {
            Page::Overview => {
                self.agents = reader.agent_stats(&self.filter)?;
                sort_agents(&mut self.agents, self.overview.sort);
                clamp_sel(&mut self.overview.table, self.agents.len());
            }
            Page::Trend => {
                self.trend = reader.trend(&self.filter)?;
            }
            Page::Requests => {
                let max_page = self.max_request_page();
                if self.request_state.page > max_page {
                    self.request_state.page = max_page;
                }
                self.requests = reader.request_logs(
                    &self.filter,
                    self.request_state.status,
                    self.request_state.page,
                    self.page_size,
                )?;
                clamp_sel(&mut self.request_state.table, self.requests.rows.len());
            }
            Page::Providers => {
                self.providers = reader.provider_stats(&self.filter)?;
                sort_providers(&mut self.providers, self.provider_state.sort);
                clamp_sel(&mut self.provider_state.table, self.providers.len());
            }
            Page::Models => {
                self.models = reader.model_stats(&self.filter)?;
                sort_models(&mut self.models, self.model_state.sort);
                clamp_sel(&mut self.model_state.table, self.models.len());
            }
        }
        Ok(())
    }

    /// Reload cascading filter options (range/agent/provider changed, or manual).
    pub fn refresh_options(&mut self, reader: &Reader) {
        match reader.filter_options(&self.filter) {
            Ok(o) => self.options = o,
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    fn set_error(&mut self, msg: &str) {
        self.status = if msg.contains("locked") || msg.contains("busy") {
            "db busy (cc-switch writing) — retrying…".into()
        } else {
            format!("error: {msg}")
        };
        self.status_error = true;
        if let Ok(fs) = std::fs::metadata(&self.meta.db_path) {
            self.last_mtime = fs.modified().ok();
        }
    }

    fn max_request_page(&self) -> u32 {
        let total = self.requests.total;
        if total == 0 {
            0
        } else {
            (total - 1) / self.page_size.max(1)
        }
    }

    // ---- actions ----

    pub fn apply(&mut self, action: UiAction, reader: &Reader) {
        use UiAction::*;
        match action {
            Quit => self.quit = true,
            Refresh => {
                self.refresh_options(reader);
                self.refresh_active(reader);
            }
            SetPage(p) => {
                self.page = p;
                self.refresh_active(reader);
            }
            NextPage => {
                let i = Page::ORDER.iter().position(|p| *p == self.page).unwrap_or(0);
                self.page = Page::ORDER[(i + 1) % Page::ORDER.len()];
                self.refresh_active(reader);
            }
            PrevPage => {
                let i = Page::ORDER.iter().position(|p| *p == self.page).unwrap_or(0);
                let n = Page::ORDER.len();
                self.page = Page::ORDER[(i + n - 1) % n];
                self.refresh_active(reader);
            }
            SetRange(r) => {
                self.filter.range = r;
                self.reset_request_page();
                self.refresh_options(reader);
                self.refresh_active(reader);
            }
            CycleRange => {
                let i = Range::ORDER.iter().position(|r| *r == self.filter.range).unwrap_or(0);
                self.filter.range = Range::ORDER[(i + 1) % Range::ORDER.len()];
                self.reset_request_page();
                self.refresh_options(reader);
                self.refresh_active(reader);
            }
            SetAgent(a) => {
                self.filter.agent = a;
                self.filter.provider = None;
                self.filter.model = None;
                self.reset_request_page();
                self.refresh_options(reader);
                self.refresh_active(reader);
            }
            SetProvider(p) => {
                self.filter.provider = p;
                self.filter.model = None;
                self.reset_request_page();
                self.refresh_options(reader);
                self.refresh_active(reader);
            }
            SetModel(m) => {
                self.filter.model = m;
                self.reset_request_page();
                self.refresh_active(reader);
            }
            SelectNext => self.move_sel(1),
            SelectPrev => self.move_sel(-1),
            PageUp => self.move_sel(-(self.page_size as isize)),
            PageDown => self.move_sel(self.page_size as isize),
            SelectFirst => self.set_sel(0),
            SelectLast => self.set_sel(usize::MAX),
            ActivateRow(i) => self.activate_row(i, reader),
            CycleTrendMetric => {
                let i = TrendMetric::ORDER.iter().position(|m| *m == self.trend_state.metric).unwrap_or(0);
                self.trend_state.metric = TrendMetric::ORDER[(i + 1) % TrendMetric::ORDER.len()];
            }
            RequestNextPage => {
                if self.request_state.page < self.max_request_page() {
                    self.request_state.page += 1;
                    self.request_state.table.select(Some(0));
                    self.refresh_active(reader);
                }
            }
            RequestPrevPage => {
                if self.request_state.page > 0 {
                    self.request_state.page -= 1;
                    self.request_state.table.select(Some(0));
                    self.refresh_active(reader);
                }
            }
            CycleRequestStatus => {
                self.request_state.status = match self.request_state.status {
                    None => Some(200),
                    Some(200) => Some(500),
                    _ => None,
                };
                self.reset_request_page();
                self.refresh_active(reader);
            }
            CycleSort => {
                let bump = |s: &mut SortKey| {
                    let i = SortKey::ORDER.iter().position(|k| *k == *s).unwrap_or(0);
                    *s = SortKey::ORDER[(i + 1) % SortKey::ORDER.len()];
                };
                match self.page {
                    Page::Overview => {
                        bump(&mut self.overview.sort);
                        sort_agents(&mut self.agents, self.overview.sort);
                    }
                    Page::Providers => {
                        bump(&mut self.provider_state.sort);
                        sort_providers(&mut self.providers, self.provider_state.sort);
                    }
                    Page::Models => {
                        bump(&mut self.model_state.sort);
                        sort_models(&mut self.models, self.model_state.sort);
                    }
                    _ => {}
                }
            }
            OpenAgentPicker => {
                self.refresh_options(reader);
                self.popup = Popup::Agent;
                self.popup_sel = 0;
            }
            OpenProviderPicker => {
                self.refresh_options(reader);
                self.popup = Popup::Provider;
                self.popup_sel = 0;
            }
            OpenModelPicker => {
                self.refresh_options(reader);
                self.popup = Popup::Model;
                self.popup_sel = 0;
            }
            ClosePopup => self.popup = Popup::None,
            PopupNext => self.move_popup(1),
            PopupPrev => self.move_popup(-1),
            PopupConfirm => self.confirm_popup(reader),
            Noop => {}
        }
    }

    fn reset_request_page(&mut self) {
        self.request_state.page = 0;
        self.request_state.table.select(Some(0));
    }

    fn current_len(&self) -> usize {
        match self.page {
            Page::Overview => self.agents.len(),
            Page::Trend => 0,
            Page::Requests => self.requests.rows.len(),
            Page::Providers => self.providers.len(),
            Page::Models => self.models.len(),
        }
    }

    fn sel_state(&mut self) -> Option<&mut TableState> {
        match self.page {
            Page::Overview => Some(&mut self.overview.table),
            Page::Requests => Some(&mut self.request_state.table),
            Page::Providers => Some(&mut self.provider_state.table),
            Page::Models => Some(&mut self.model_state.table),
            Page::Trend => None,
        }
    }

    fn move_sel(&mut self, delta: isize) {
        let len = self.current_len();
        if len == 0 {
            return;
        }
        if let Some(st) = self.sel_state() {
            let cur = st.selected().unwrap_or(0) as isize;
            let mut nxt = cur + delta;
            if nxt < 0 {
                nxt = 0;
            }
            if nxt > len as isize - 1 {
                nxt = len as isize - 1;
            }
            st.select(Some(nxt as usize));
        }
    }

    fn set_sel(&mut self, i: usize) {
        let len = self.current_len();
        if len == 0 {
            return;
        }
        let idx = i.min(len - 1);
        if let Some(st) = self.sel_state() {
            st.select(Some(idx));
        }
    }

    fn activate_row(&mut self, i: usize, reader: &Reader) {
        self.set_sel(i);
        match self.page {
            Page::Overview => {
                if let Some(a) = self.agents.get(i) {
                    let name = a.agent.clone();
                    self.apply(UiAction::SetAgent(Some(name)), reader);
                }
            }
            Page::Models => {
                if let Some(m) = self.models.get(i) {
                    let name = m.model.clone();
                    self.apply(UiAction::SetModel(Some(name)), reader);
                }
            }
            Page::Providers => {
                if let Some(p) = self.providers.get(i) {
                    let key = ProviderKey {
                        app_type: p.app_type.clone(),
                        provider_id: p.provider_id.clone(),
                        display_name: p.provider_name.clone(),
                    };
                    self.apply(UiAction::SetProvider(Some(key)), reader);
                }
            }
            _ => {}
        }
    }

    // ---- popup ----

    pub fn popup_items(&self) -> Vec<String> {
        match self.popup {
            Popup::Agent => std::iter::once("All (agents)".to_string()).chain(self.options.agents.iter().cloned()).collect(),
            Popup::Provider => std::iter::once("All (providers)".to_string())
                .chain(self.options.providers.iter().map(|p| format!("{} · {}", p.display_name, p.app_type)))
                .collect(),
            Popup::Model => std::iter::once("All (models)".to_string()).chain(self.options.models.iter().cloned()).collect(),
            Popup::None => Vec::new(),
        }
    }

    fn move_popup(&mut self, delta: isize) {
        let len = self.popup_items().len();
        if len == 0 {
            return;
        }
        let cur = self.popup_sel as isize;
        let mut nxt = cur + delta;
        if nxt < 0 {
            nxt = 0;
        }
        if nxt > len as isize - 1 {
            nxt = len as isize - 1;
        }
        self.popup_sel = nxt as usize;
    }

    fn confirm_popup(&mut self, reader: &Reader) {
        let sel = self.popup_sel;
        match self.popup {
            Popup::Agent => {
                let a = if sel == 0 { None } else { self.options.agents.get(sel - 1).cloned() };
                self.popup = Popup::None;
                self.apply(UiAction::SetAgent(a), reader);
            }
            Popup::Provider => {
                let p = if sel == 0 { None } else { self.options.providers.get(sel - 1).cloned() };
                self.popup = Popup::None;
                self.apply(UiAction::SetProvider(p), reader);
            }
            Popup::Model => {
                let m = if sel == 0 { None } else { self.options.models.get(sel - 1).cloned() };
                self.popup = Popup::None;
                self.apply(UiAction::SetModel(m), reader);
            }
            Popup::None => {}
        }
    }
}

fn clamp_sel(st: &mut TableState, len: usize) {
    if len == 0 {
        st.select(None);
    } else if let Some(s) = st.selected() {
        if s >= len {
            st.select(Some(len - 1));
        }
    } else {
        st.select(Some(0));
    }
}

pub fn sort_agents(v: &mut [AgentStat], key: SortKey) {
    match key {
        SortKey::Cost => v.sort_by(|a, b| b.cost.total_cmp(&a.cost)),
        SortKey::Tokens => v.sort_by_key(|s| std::cmp::Reverse(s.total_tokens())),
        SortKey::Reqs => v.sort_by_key(|s| std::cmp::Reverse(s.reqs)),
        SortKey::CacheHit => v.sort_by(|a, b| b.cache_hit().total_cmp(&a.cache_hit())),
        SortKey::Name => v.sort_by(|a, b| a.agent.cmp(&b.agent)),
    }
}

pub fn sort_models(v: &mut [ModelStat], key: SortKey) {
    match key {
        SortKey::Cost => v.sort_by(|a, b| b.cost.total_cmp(&a.cost)),
        SortKey::Tokens => v.sort_by_key(|s| std::cmp::Reverse(s.total_tokens())),
        SortKey::Reqs => v.sort_by_key(|s| std::cmp::Reverse(s.reqs)),
        SortKey::CacheHit => v.sort_by(|a, b| b.cache_hit().total_cmp(&a.cache_hit())),
        SortKey::Name => v.sort_by(|a, b| a.model.cmp(&b.model)),
    }
}

pub fn sort_providers(v: &mut [ProviderStat], key: SortKey) {
    match key {
        SortKey::Cost => v.sort_by(|a, b| b.cost.total_cmp(&a.cost)),
        SortKey::Tokens => v.sort_by_key(|s| std::cmp::Reverse(s.total_tokens)),
        SortKey::Reqs => v.sort_by_key(|s| std::cmp::Reverse(s.reqs)),
        SortKey::CacheHit => v.sort_by(|a, b| b.success_rate().total_cmp(&a.success_rate())),
        SortKey::Name => v.sort_by(|a, b| a.provider_name.cmp(&b.provider_name)),
    }
}
