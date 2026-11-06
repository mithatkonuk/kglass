//! Application state and the pure `update` function that drives it.
//!
//! All state changes go through [`App::update`], which performs no I/O. In phase 1
//! events come from the terminal and a timer; later phases add API results as
//! further `AppEvent` variants.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::derive::images_in_use;
use crate::fake::FakeLogs;
use crate::model::{
    ClusterSnapshot, ImageUsage, ResourceKind, ResourceRef, StorageItem, StorageKind,
};
use crate::state::{ListCursor, LogBuffer};
use crate::text::SafeText;
use crate::ui::hits::{Hit, HitMap};

/// Maximum buffered log lines (design D5).
pub const LOG_CAP: usize = 5000;
/// Initial log tail length (design D5).
pub const LOG_TAIL: usize = 500;
/// Rows moved per mouse wheel notch.
const WHEEL_STEP: isize = 3;
/// Rows moved by PgUp/PgDn.
const PAGE: isize = 10;
/// A simulated log line arrives every this many ticks (≈437ms at a 250ms tick).
const TICKS_PER_LINE: u64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List(ResourceKind),
    Viewer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerTab {
    Logs,
    Summary,
    Events,
    Yaml,
    Containers,
    Details,
    Workloads,
    Endpoints,
    MountedBy,
    Claim,
    StorageDetails,
}

impl ViewerTab {
    pub fn title(self) -> &'static str {
        match self {
            ViewerTab::Logs => "Logs",
            ViewerTab::Summary => "Summary",
            ViewerTab::Events => "Events",
            ViewerTab::Yaml => "YAML",
            ViewerTab::Containers => "Containers",
            ViewerTab::Details => "Details",
            ViewerTab::Workloads => "Workloads",
            ViewerTab::Endpoints => "Endpoints",
            ViewerTab::MountedBy => "Mounted by",
            ViewerTab::Claim => "Claim",
            ViewerTab::StorageDetails => "Storage details",
        }
    }
}

/// Viewer tabs for a resource (design §4.4); the first one is the default.
pub fn tabs_for(r: &ResourceRef) -> &'static [ViewerTab] {
    use ViewerTab::*;
    match r {
        ResourceRef::Pod(_) => &[Logs, Summary, Events, Yaml, Containers],
        ResourceRef::Image(_) => &[Details, Workloads],
        ResourceRef::Service(_) => &[Details, Endpoints, Events, Yaml],
        ResourceRef::Pvc(_) => &[Details, MountedBy, Events, Yaml],
        ResourceRef::Pv(_) => &[Details, Claim, StorageDetails, Yaml],
    }
}

/// Startup options from the command line and environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppOptions {
    pub plain: bool,
    pub context: Option<String>,
    pub namespace: Option<String>,
    pub all_namespaces: bool,
}

const FOCUS_ORDER: [Focus; 5] = [
    Focus::List(ResourceKind::Pods),
    Focus::List(ResourceKind::Images),
    Focus::List(ResourceKind::Services),
    Focus::List(ResourceKind::Storage),
    Focus::Viewer,
];

#[derive(Debug)]
pub struct App {
    pub snapshot: ClusterSnapshot,
    pub images: Vec<ImageUsage>,
    /// Indexed by `ResourceKind::index()`.
    pub cursors: [ListCursor; 4],
    pub focus: Focus,
    /// The resource shown in the viewer.
    pub selected: Option<ResourceRef>,
    /// Index into `current_tabs()`.
    pub tab: usize,
    /// Index into `current_containers()`.
    pub container: usize,
    pub logs: LogBuffer,
    /// Scroll offset for non-log viewer tabs.
    pub viewer_scroll: u16,
    /// Text rows available in the viewer body, recorded by the renderer.
    pub viewer_rows: usize,
    pub zoom: bool,
    pub help: bool,
    pub quit: bool,
    pub plain: bool,
    pub all_namespaces: bool,
    pub hits: HitMap,
    pub size: (u16, u16),
    fake_logs: FakeLogs,
    ticks: u64,
    last_list: ResourceKind,
}

impl App {
    pub fn new(mut snapshot: ClusterSnapshot, opts: AppOptions) -> App {
        if let Some(ctx) = &opts.context {
            snapshot.context = SafeText::from(ctx.as_str());
        }
        if let Some(ns) = &opts.namespace {
            snapshot.namespace = SafeText::from(ns.as_str());
        }
        let images = images_in_use(&snapshot.pods);
        let mut app = App {
            snapshot,
            images,
            cursors: Default::default(),
            focus: Focus::List(ResourceKind::Pods),
            selected: None,
            tab: 0,
            container: 0,
            logs: LogBuffer::new(LOG_CAP),
            viewer_scroll: 0,
            viewer_rows: 20,
            zoom: false,
            help: false,
            quit: false,
            plain: opts.plain,
            all_namespaces: opts.all_namespaces,
            hits: HitMap::default(),
            size: (0, 0),
            fake_logs: FakeLogs::new(),
            ticks: 0,
            last_list: ResourceKind::Pods,
        };
        for kind in ResourceKind::ALL {
            let len = app.row_count(kind);
            app.cursors[kind.index()].set_len(len);
        }
        if let Some(first) = app.row_ref(ResourceKind::Pods, 0) {
            app.select(first);
        }
        app
    }

    pub fn update(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Mouse(mouse) => self.on_mouse(mouse),
            AppEvent::Resize(w, h) => self.size = (w, h),
            AppEvent::Tick => self.on_tick(),
        }
    }

    pub fn row_count(&self, kind: ResourceKind) -> usize {
        match kind {
            ResourceKind::Pods => self.snapshot.pods.len(),
            ResourceKind::Images => self.images.len(),
            ResourceKind::Services => self.snapshot.services.len(),
            ResourceKind::Storage => self.snapshot.storage.len(),
        }
    }

    /// Storage rows in display order: claims first, then volumes.
    pub fn storage_rows(&self) -> impl Iterator<Item = &StorageItem> {
        let of = |k| self.snapshot.storage.iter().filter(move |s| s.kind == k);
        of(StorageKind::Pvc).chain(of(StorageKind::Pv))
    }

    pub fn row_ref(&self, kind: ResourceKind, idx: usize) -> Option<ResourceRef> {
        match kind {
            ResourceKind::Pods => self
                .snapshot
                .pods
                .get(idx)
                .map(|p| ResourceRef::Pod(p.name.clone())),
            ResourceKind::Images => self
                .images
                .get(idx)
                .map(|i| ResourceRef::Image(i.raw.clone())),
            ResourceKind::Services => self
                .snapshot
                .services
                .get(idx)
                .map(|s| ResourceRef::Service(s.name.clone())),
            ResourceKind::Storage => self.storage_rows().nth(idx).map(|s| match s.kind {
                StorageKind::Pvc => ResourceRef::Pvc(s.name.clone()),
                StorageKind::Pv => ResourceRef::Pv(s.name.clone()),
            }),
        }
    }

    pub fn current_tabs(&self) -> &'static [ViewerTab] {
        self.selected.as_ref().map_or(&[], tabs_for)
    }

    pub fn current_tab(&self) -> Option<ViewerTab> {
        self.current_tabs().get(self.tab).copied()
    }

    pub fn selected_pod(&self) -> Option<&crate::model::Pod> {
        match &self.selected {
            Some(ResourceRef::Pod(name)) => self.snapshot.pods.iter().find(|p| &p.name == name),
            _ => None,
        }
    }

    /// Containers of the selected pod: regular containers first, then init containers.
    pub fn current_containers(&self) -> Vec<&SafeText> {
        let Some(pod) = self.selected_pod() else {
            return Vec::new();
        };
        let regular = pod.containers.iter().filter(|c| !c.init);
        let init = pod.containers.iter().filter(|c| c.init);
        regular.chain(init).map(|c| &c.name).collect()
    }

    pub fn current_container(&self) -> Option<&SafeText> {
        self.current_containers().get(self.container).copied()
    }

    /// Show `r` in the viewer on its default tab, replacing any log stream.
    pub fn select(&mut self, r: ResourceRef) {
        self.selected = Some(r);
        self.tab = 0;
        self.container = 0;
        self.viewer_scroll = 0;
        self.restart_logs();
    }

    /// Focus a pane (also used by mouse handling).
    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        if let Focus::List(kind) = focus {
            self.last_list = kind;
        }
    }

    /// The list shown in the narrow layout: the focused list, or the last one
    /// focused while the viewer has focus.
    pub fn visible_list(&self) -> ResourceKind {
        match self.focus {
            Focus::List(kind) => kind,
            Focus::Viewer => self.last_list,
        }
    }

    fn restart_logs(&mut self) {
        self.logs.clear();
        let lines = match (self.selected_pod(), self.current_container()) {
            (Some(pod), Some(container)) => {
                self.fake_logs.initial(pod, container.as_str(), LOG_TAIL)
            }
            _ => Vec::new(),
        };
        self.logs.extend(lines.into_iter().map(SafeText::from));
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks % TICKS_PER_LINE != 0 {
            return;
        }
        let Some(pod) = self.selected_pod().cloned() else {
            return;
        };
        let Some(container) = self.current_container().cloned() else {
            return;
        };
        if let Some(line) = self.fake_logs.next_line(&pod, container.as_str()) {
            self.logs.push(SafeText::from(line));
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if (ctrl && key.code == KeyCode::Char('c')) || key.code == KeyCode::Char('q') {
            self.quit = true;
            return;
        }
        if self.help {
            if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                self.help = false;
            }
            return;
        }
        match key.code {
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc => self.zoom = false,
            KeyCode::Char('z') => self.zoom = !self.zoom,
            KeyCode::Up | KeyCode::Char('k') => self.move_focused(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_focused(1),
            KeyCode::PageUp => self.move_focused(-PAGE),
            KeyCode::PageDown => self.move_focused(PAGE),
            KeyCode::Enter => self.activate_cursor(),
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Left => self.cycle_tab(-1),
            KeyCode::Right => self.cycle_tab(1),
            KeyCode::Char('f') => {
                let on = !self.logs.follow();
                self.logs.set_follow(on);
            }
            KeyCode::Char('c') => self.cycle_container(),
            KeyCode::Char(d @ '1'..='4') => {
                if let Some(kind) = ResourceKind::from_number(d as u8 - b'0') {
                    self.set_focus(Focus::List(kind));
                }
            }
            _ => {}
        }
    }

    /// Move the list cursor, or scroll the viewer, by `delta` rows.
    pub fn move_focused(&mut self, delta: isize) {
        match self.focus {
            Focus::List(kind) => self.cursors[kind.index()].move_by(delta),
            Focus::Viewer => self.scroll_viewer(delta),
        }
    }

    pub fn scroll_viewer(&mut self, delta: isize) {
        if self.current_tab() == Some(ViewerTab::Logs) {
            self.logs.scroll_by(delta, self.viewer_rows);
        } else {
            self.viewer_scroll = self
                .viewer_scroll
                .saturating_add_signed(delta.clamp(i16::MIN.into(), i16::MAX.into()) as i16);
        }
    }

    /// Select the row under the focused list's cursor.
    pub fn activate_cursor(&mut self) {
        let Focus::List(kind) = self.focus else {
            return;
        };
        let Some(idx) = self.cursors[kind.index()].selected() else {
            return;
        };
        if let Some(r) = self.row_ref(kind, idx) {
            self.select(r);
        }
    }

    /// Mouse input, resolved against the regions recorded by the last frame.
    fn on_mouse(&mut self, mouse: MouseEvent) {
        if self.help {
            return;
        }
        let Some(hit) = self.hits.hit(mouse.column, mouse.row) else {
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Hit::ListRow(kind, idx) => {
                    self.set_focus(Focus::List(kind));
                    self.cursors[kind.index()].select(idx);
                    self.activate_cursor();
                }
                Hit::ListPane(kind) | Hit::ListTab(kind) => self.set_focus(Focus::List(kind)),
                Hit::ViewerTab(idx) => self.set_tab(idx),
                Hit::Viewer => self.set_focus(Focus::Viewer),
            },
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown {
                    WHEEL_STEP
                } else {
                    -WHEEL_STEP
                };
                match hit {
                    Hit::ListRow(kind, _) | Hit::ListPane(kind) => {
                        self.cursors[kind.index()].move_by(delta);
                    }
                    Hit::Viewer | Hit::ViewerTab(_) => self.scroll_viewer(delta),
                    Hit::ListTab(_) => {}
                }
            }
            _ => {}
        }
    }

    fn cycle_focus(&mut self, step: isize) {
        let pos = FOCUS_ORDER
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        let n = FOCUS_ORDER.len() as isize;
        self.set_focus(FOCUS_ORDER[(pos as isize + step).rem_euclid(n) as usize]);
    }

    /// Switch to viewer tab `idx` (ignored when out of range).
    pub fn set_tab(&mut self, idx: usize) {
        if idx < self.current_tabs().len() {
            self.tab = idx;
            self.viewer_scroll = 0;
        }
    }

    fn cycle_tab(&mut self, step: isize) {
        let n = self.current_tabs().len() as isize;
        if n > 0 {
            self.set_tab((self.tab as isize + step).rem_euclid(n) as usize);
        }
    }

    fn cycle_container(&mut self) {
        let n = self.current_containers().len();
        if n > 1 && self.current_tab() == Some(ViewerTab::Logs) {
            self.container = (self.container + 1) % n;
            self.restart_logs();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::sample_snapshot;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn app() -> App {
        App::new(sample_snapshot(), AppOptions::default())
    }

    fn pod(name: &str) -> Option<ResourceRef> {
        Some(ResourceRef::Pod(name.into()))
    }

    #[test]
    fn starts_like_pdf_page1() {
        let a = app();
        assert_eq!(a.focus, Focus::List(ResourceKind::Pods));
        assert_eq!(a.selected, pod("api-7c9d8f-x2k4"));
        assert_eq!(a.current_tab(), Some(ViewerTab::Logs));
        assert_eq!(a.logs.len(), 34);
    }

    #[test]
    fn cursor_moves_but_selection_waits_for_enter() {
        let mut a = app();
        a.update(key(KeyCode::Down));
        assert_eq!(a.selected, pod("api-7c9d8f-x2k4"));
        a.update(key(KeyCode::Enter));
        assert_eq!(a.selected, pod("api-7c9d8f-q8n1"));
    }

    #[test]
    fn selecting_service_opens_details() {
        let mut a = app();
        a.update(key(KeyCode::Char('3')));
        a.update(key(KeyCode::Down));
        a.update(key(KeyCode::Down));
        a.update(key(KeyCode::Enter));
        assert_eq!(a.selected, Some(ResourceRef::Service("billing".into())));
        assert_eq!(a.current_tab(), Some(ViewerTab::Details));
    }

    #[test]
    fn storage_rows_list_pvcs_then_pvs() {
        let a = app();
        assert_eq!(
            a.row_ref(ResourceKind::Storage, 3),
            Some(ResourceRef::Pv("pvc-3e1a9c52-0c09".into()))
        );
        assert_eq!(a.row_count(ResourceKind::Storage), 5);
        assert_eq!(a.row_ref(ResourceKind::Storage, 5), None);
    }

    #[test]
    fn tabs_wrap_both_ways() {
        let mut a = app();
        a.update(key(KeyCode::Left));
        assert_eq!(a.current_tab(), Some(ViewerTab::Containers));
        a.update(key(KeyCode::Right));
        assert_eq!(a.current_tab(), Some(ViewerTab::Logs));
    }

    #[test]
    fn focus_cycles_with_tab_and_backtab() {
        let mut a = app();
        for expected in [
            Focus::List(ResourceKind::Images),
            Focus::List(ResourceKind::Services),
            Focus::List(ResourceKind::Storage),
            Focus::Viewer,
            Focus::List(ResourceKind::Pods),
        ] {
            a.update(key(KeyCode::Tab));
            assert_eq!(a.focus, expected);
        }
        a.update(key(KeyCode::BackTab));
        assert_eq!(a.focus, Focus::Viewer);
    }

    #[test]
    fn ticks_stream_and_switching_pod_replaces_stream() {
        let mut a = app();
        for _ in 0..8 {
            a.update(AppEvent::Tick);
        }
        assert_eq!(a.logs.len(), 38);
        a.update(key(KeyCode::Down));
        a.update(key(KeyCode::Enter));
        assert_eq!(a.logs.len(), 34);
    }

    #[test]
    fn container_cycle_refills_logs() {
        let mut a = app();
        a.update(key(KeyCode::Char('c')));
        assert_eq!(
            a.current_container().map(|c| c.to_string()),
            Some("envoy".into())
        );
        assert!(a.logs.visible(1).next().unwrap().contains("envoy"));
        a.update(key(KeyCode::Char('c')));
        assert_eq!(
            a.current_container().map(|c| c.to_string()),
            Some("api".into())
        );
    }

    #[test]
    fn follow_toggle_and_scrolling_in_viewer() {
        let mut a = app();
        a.update(key(KeyCode::Char('f')));
        assert!(!a.logs.follow());
        a.update(key(KeyCode::Char('f')));
        a.update(key(KeyCode::Tab));
        a.update(key(KeyCode::Tab));
        a.update(key(KeyCode::Tab));
        a.update(key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Viewer);
        a.update(key(KeyCode::Up));
        assert!(!a.logs.follow(), "scrolling up leaves follow mode");
    }

    #[test]
    fn help_swallows_keys() {
        let mut a = app();
        a.update(key(KeyCode::Char('?')));
        assert!(a.help);
        a.update(key(KeyCode::Char('3')));
        assert_eq!(a.focus, Focus::List(ResourceKind::Pods));
        a.update(key(KeyCode::Esc));
        assert!(!a.help);
    }

    #[test]
    fn esc_leaves_zoom() {
        let mut a = app();
        a.update(key(KeyCode::Char('z')));
        assert!(a.zoom);
        a.update(key(KeyCode::Esc));
        assert!(!a.zoom);
    }

    #[test]
    fn ctrl_c_and_q_quit() {
        let mut a = app();
        a.update(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(a.quit);
        let mut b = app();
        b.update(key(KeyCode::Char('q')));
        assert!(b.quit);
    }

    #[test]
    fn key_release_is_ignored() {
        let mut a = app();
        let mut ev = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        ev.kind = ratatui::crossterm::event::KeyEventKind::Release;
        a.update(AppEvent::Key(ev));
        assert_eq!(a.cursors[0].selected(), Some(0));
    }

    #[test]
    fn empty_services_enter_is_noop() {
        let mut s = sample_snapshot();
        s.services.clear();
        let mut a = App::new(s, AppOptions::default());
        a.update(key(KeyCode::Char('3')));
        a.update(key(KeyCode::Down));
        a.update(key(KeyCode::Enter));
        assert_eq!(a.selected, pod("api-7c9d8f-x2k4"));
    }

    #[test]
    fn empty_cluster_selects_nothing() {
        let mut s = sample_snapshot();
        s.pods.clear();
        let mut a = App::new(s, AppOptions::default());
        assert_eq!(a.selected, None);
        assert_eq!(a.current_tab(), None);
        a.update(key(KeyCode::Right));
        a.update(key(KeyCode::Char('c')));
        a.update(AppEvent::Tick);
        assert!(a.logs.is_empty());
    }

    #[test]
    fn cli_overrides_header_labels() {
        let a = App::new(
            sample_snapshot(),
            AppOptions {
                context: Some("prod".into()),
                namespace: Some("\x1b[31mshop".into()),
                ..AppOptions::default()
            },
        );
        assert_eq!(a.snapshot.context.as_str(), "prod");
        assert_eq!(a.snapshot.namespace.as_str(), "shop");
    }

    #[test]
    fn tabs_per_resource_match_design() {
        use ViewerTab::*;
        assert_eq!(
            tabs_for(&ResourceRef::Pod("x".into())),
            [Logs, Summary, Events, Yaml, Containers]
        );
        assert_eq!(
            tabs_for(&ResourceRef::Image("x".into())),
            [Details, Workloads]
        );
        assert_eq!(
            tabs_for(&ResourceRef::Service("x".into())),
            [Details, Endpoints, Events, Yaml]
        );
        assert_eq!(
            tabs_for(&ResourceRef::Pvc("x".into())),
            [Details, MountedBy, Events, Yaml]
        );
        assert_eq!(
            tabs_for(&ResourceRef::Pv("x".into())),
            [Details, Claim, StorageDetails, Yaml]
        );
    }
}
