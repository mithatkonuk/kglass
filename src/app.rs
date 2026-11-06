//! Application state and the pure `update` function that drives it.
//!
//! All state changes go through [`App::update`], which performs no I/O. Side effects
//! (API calls, log streams) are queued as [`Command`]s for the runtime to execute;
//! their results come back as [`AppEvent::Data`].

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::cluster::config::ContextInfo;
use crate::cluster::fetch::{FetchError, Lists};
use crate::command::Command;
use crate::derive::images_in_use;
use crate::fake::FakeLogs;
use crate::model::{
    ClusterSnapshot, ImageUsage, ResourceKind, ResourceRef, StorageItem, StorageKind,
};
use crate::state::{ListCursor, LogBuffer, Picker, PickerItem, PickerKind};
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
/// Live lists refresh every this many ticks (10s at a 250ms tick, design D5).
pub const REFRESH_TICKS: u64 = 40;

#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    Data(DataEvent),
}

/// Results of [`Command`]s. Each carries the generation or session it was
/// requested for, so late answers for an old target or selection are dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum DataEvent {
    Lists {
        generation: u64,
        lists: Box<Lists>,
    },
    Version {
        generation: u64,
        result: Result<SafeText, FetchError>,
    },
    LogLine {
        session: u64,
        line: SafeText,
    },
    LogEnded {
        session: u64,
        error: Option<FetchError>,
    },
    Yaml {
        generation: u64,
        target: ResourceRef,
        result: Result<String, FetchError>,
    },
    Namespaces(Result<Vec<SafeText>, FetchError>),
}

/// Where the data comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Sample,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    /// Built-in sample data; never connected.
    Sample,
    Connecting,
    Connected {
        version: SafeText,
    },
    Disconnected {
        error: SafeText,
    },
}

/// Load state of one list pane. Failed panes keep their last good rows (stale).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneStatus {
    Loading {
        since_tick: u64,
    },
    Ready,
    /// Rows shown, but part of the data could not be fetched.
    Partial(FetchError),
    Failed(FetchError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogState {
    Idle,
    Streaming,
    Ended(SafeText),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YamlState {
    Loading,
    Ready(String),
    Failed(FetchError),
}

/// Context and namespace a live app starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTarget {
    pub context: SafeText,
    pub namespace: SafeText,
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
    pub connection: Connection,
    /// Indexed by `ResourceKind::index()`. Images follow the pods pane.
    pub panes: [PaneStatus; 4],
    pub log_state: LogState,
    /// Shown instead of the viewer content, e.g. when the selection was deleted.
    pub viewer_notice: Option<SafeText>,
    /// Open context or namespace picker; captures keys while open.
    pub picker: Option<Picker>,
    /// List whose filter is being typed; captures keys while set.
    pub filter_editing: Option<ResourceKind>,
    /// Name filters per list (case-insensitive substring), indexed by `ResourceKind::index()`.
    filters: [String; 4],
    contexts: Vec<ContextInfo>,
    source: Source,
    commands: Vec<Command>,
    /// Bumped when the target (context/namespace) changes.
    generation: u64,
    /// Bumped whenever the log stream is replaced or stopped.
    log_session: u64,
    refresh_in_flight: bool,
    last_refresh_tick: u64,
    yaml: Option<(ResourceRef, YamlState)>,
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
            connection: Connection::Sample,
            panes: std::array::from_fn(|_| PaneStatus::Ready),
            log_state: LogState::Idle,
            viewer_notice: None,
            picker: None,
            filter_editing: None,
            filters: Default::default(),
            contexts: Vec::new(),
            source: Source::Sample,
            commands: Vec::new(),
            generation: 0,
            log_session: 0,
            refresh_in_flight: false,
            last_refresh_tick: 0,
            yaml: None,
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

    /// An app showing a real cluster. It starts empty and asks for a refresh.
    pub fn new_live(target: LiveTarget, plain: bool) -> App {
        let snapshot = ClusterSnapshot {
            context: target.context,
            namespace: target.namespace,
            user: SafeText::default(),
            api_version: SafeText::default(),
            distro: SafeText::default(),
            pods: Vec::new(),
            services: Vec::new(),
            storage: Vec::new(),
            events: Vec::new(),
        };
        let mut app = App::new(
            snapshot,
            AppOptions {
                plain,
                ..AppOptions::default()
            },
        );
        app.source = Source::Live;
        app.connection = Connection::Connecting;
        app.panes = std::array::from_fn(|_| PaneStatus::Loading { since_tick: 0 });
        app.request_refresh();
        app
    }

    pub fn update(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Mouse(mouse) => self.on_mouse(mouse),
            AppEvent::Resize(w, h) => self.size = (w, h),
            AppEvent::Tick => self.on_tick(),
            AppEvent::Data(data) => self.on_data(data),
        }
    }

    /// Commands queued since the last call, for the runtime to execute.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn is_live(&self) -> bool {
        self.source == Source::Live
    }

    /// Ask for fresh lists, unless one refresh is already in flight.
    pub fn request_refresh(&mut self) {
        if !self.is_live() || self.refresh_in_flight {
            return;
        }
        self.refresh_in_flight = true;
        self.last_refresh_tick = self.ticks;
        self.commands.push(Command::Refresh {
            generation: self.generation,
        });
    }

    /// True when the pane shows rows from an earlier refresh because the last one failed.
    pub fn is_stale(&self, kind: ResourceKind) -> bool {
        matches!(self.panes[kind.index()], PaneStatus::Failed(_)) && self.row_count(kind) > 0
    }

    /// YAML state for the selected resource, if it was requested.
    pub fn yaml_state(&self) -> Option<&YamlState> {
        match (&self.yaml, &self.selected) {
            (Some((target, state)), Some(selected)) if target == selected => Some(state),
            _ => None,
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    fn on_data(&mut self, data: DataEvent) {
        match data {
            DataEvent::Lists { generation, lists } if generation == self.generation => {
                self.apply_lists(*lists);
            }
            DataEvent::Version { generation, result } if generation == self.generation => {
                self.connection = match result {
                    Ok(version) => Connection::Connected { version },
                    Err(e) => Connection::Disconnected {
                        error: SafeText::from(e.to_string()),
                    },
                };
            }
            DataEvent::LogLine { session, line } if session == self.log_session => {
                self.logs.push(line);
            }
            DataEvent::LogEnded { session, error } if session == self.log_session => {
                let reason = error.map_or_else(|| "stream closed".to_string(), |e| e.to_string());
                self.log_state = LogState::Ended(SafeText::from(reason));
            }
            DataEvent::Yaml {
                generation,
                target,
                result,
            } if generation == self.generation => {
                if let Some((wanted, state)) = &mut self.yaml
                    && *wanted == target
                {
                    *state = match result {
                        Ok(text) => YamlState::Ready(text),
                        Err(e) => YamlState::Failed(e),
                    };
                }
            }
            DataEvent::Namespaces(result) => {
                if let Some(picker) = &mut self.picker
                    && picker.kind == PickerKind::Namespace
                {
                    match result {
                        Ok(names) => {
                            let current = self.snapshot.namespace.clone();
                            picker.set_items(
                                names
                                    .into_iter()
                                    .map(|name| PickerItem {
                                        current: name == current,
                                        name,
                                        detail: SafeText::default(),
                                    })
                                    .collect(),
                            );
                        }
                        Err(e) => picker.set_error(e),
                    }
                }
            }
            _ => {}
        }
    }

    fn open_context_picker(&mut self) {
        if !self.is_live() || self.contexts.is_empty() {
            return;
        }
        let current = self.snapshot.context.clone();
        let mut picker = Picker::new(PickerKind::Context);
        picker.set_items(
            self.contexts
                .iter()
                .map(|c| PickerItem {
                    name: c.name.clone(),
                    detail: SafeText::from(if c.hint.is_empty() {
                        c.server_host.to_string()
                    } else {
                        format!("{} · {}", c.hint, c.server_host)
                    }),
                    current: c.name == current,
                })
                .collect(),
        );
        self.picker = Some(picker);
    }

    fn open_namespace_picker(&mut self) {
        if !self.is_live() {
            return;
        }
        self.picker = Some(Picker::new(PickerKind::Namespace));
        self.commands.push(Command::FetchNamespaces);
    }

    fn on_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => picker.move_by(-1),
            KeyCode::Down => picker.move_by(1),
            KeyCode::Backspace => picker.backspace(),
            KeyCode::Char(c) => picker.type_char(c),
            KeyCode::Enter => {
                let (kind, chosen) = (picker.kind, picker.chosen());
                let Some(chosen) = chosen else {
                    return;
                };
                let (context, namespace) = match kind {
                    PickerKind::Context => {
                        let namespace = self
                            .contexts
                            .iter()
                            .find(|c| c.name == chosen)
                            .and_then(|c| c.namespace.clone())
                            .unwrap_or_else(|| SafeText::from("default"));
                        (chosen, namespace)
                    }
                    PickerKind::Namespace => (self.snapshot.context.clone(), chosen),
                };
                self.switch_target(context, namespace);
            }
            _ => {}
        }
    }

    fn on_filter_key(&mut self, kind: ResourceKind, key: KeyEvent) {
        let filter = &mut self.filters[kind.index()];
        match key.code {
            KeyCode::Enter => self.filter_editing = None,
            KeyCode::Esc => {
                filter.clear();
                self.filter_editing = None;
            }
            KeyCode::Backspace => {
                filter.pop();
            }
            KeyCode::Char(c) => filter.push_str(&crate::text::sanitize(&c.to_string())),
            _ => return,
        }
        let len = self.row_count(kind);
        let cursor = &mut self.cursors[kind.index()];
        cursor.set_len(len);
        cursor.select(0);
    }

    /// Show another context/namespace for this session. Clears all data, stops
    /// the log stream and starts a new generation so late answers are dropped.
    fn switch_target(&mut self, context: SafeText, namespace: SafeText) {
        self.picker = None;
        self.generation += 1;
        self.log_session += 1;
        if self.log_state == LogState::Streaming {
            self.commands.push(Command::StopLogs);
        }
        self.log_state = LogState::Idle;
        self.logs.clear();
        self.snapshot.context = context.clone();
        self.snapshot.namespace = namespace.clone();
        self.snapshot.pods.clear();
        self.snapshot.services.clear();
        self.snapshot.storage.clear();
        self.snapshot.events.clear();
        self.images.clear();
        self.selected = None;
        self.viewer_notice = None;
        self.yaml = None;
        self.tab = 0;
        self.container = 0;
        self.filters = Default::default();
        self.cursors = Default::default();
        self.panes = std::array::from_fn(|_| PaneStatus::Loading {
            since_tick: self.ticks,
        });
        self.connection = Connection::Connecting;
        self.commands
            .push(Command::SwitchTarget { context, namespace });
        self.refresh_in_flight = false;
        self.request_refresh();
    }

    fn apply_lists(&mut self, lists: Lists) {
        self.refresh_in_flight = false;
        let remembered: Vec<Option<ResourceRef>> = ResourceKind::ALL
            .iter()
            .map(|k| {
                self.cursors[k.index()]
                    .selected()
                    .and_then(|i| self.row_ref(*k, i))
            })
            .collect();

        let pods_status = match lists.pods {
            Ok(pods) => {
                self.images = images_in_use(&pods);
                self.snapshot.pods = pods;
                PaneStatus::Ready
            }
            Err(e) => PaneStatus::Failed(e),
        };
        self.panes[ResourceKind::Images.index()] = pods_status.clone();
        self.panes[ResourceKind::Pods.index()] = pods_status;
        self.panes[ResourceKind::Services.index()] = match lists.services {
            Ok(services) => {
                self.snapshot.services = services;
                PaneStatus::Ready
            }
            Err(e) => PaneStatus::Failed(e),
        };
        self.panes[ResourceKind::Storage.index()] = match lists.storage {
            Ok(storage) => {
                self.snapshot.storage = storage;
                lists
                    .storage_partial
                    .map_or(PaneStatus::Ready, PaneStatus::Partial)
            }
            Err(e) => PaneStatus::Failed(e),
        };
        if let Ok(events) = lists.events {
            self.snapshot.events = events;
        }

        for (kind, previous) in ResourceKind::ALL.into_iter().zip(remembered) {
            let len = self.row_count(kind);
            self.cursors[kind.index()].set_len(len);
            if let Some(idx) = previous.and_then(|r| self.find_row(kind, &r)) {
                self.cursors[kind.index()].select(idx);
            }
        }

        if self.selected.is_none() {
            if let Some(first) = self.row_ref(ResourceKind::Pods, 0) {
                self.select(first);
            }
        } else {
            self.check_selection_present();
            self.ensure_log_stream();
        }
    }

    fn selection_present(&self, r: &ResourceRef) -> bool {
        let name = r.name();
        match r {
            ResourceRef::Pod(_) => self.snapshot.pods.iter().any(|p| &p.name == name),
            ResourceRef::Image(_) => self.images.iter().any(|i| &i.raw == name),
            ResourceRef::Service(_) => self.snapshot.services.iter().any(|s| &s.name == name),
            ResourceRef::Pvc(_) => self
                .snapshot
                .storage
                .iter()
                .any(|s| s.kind == StorageKind::Pvc && &s.name == name),
            ResourceRef::Pv(_) => self
                .snapshot
                .storage
                .iter()
                .any(|s| s.kind == StorageKind::Pv && &s.name == name),
        }
    }

    /// Mark a deleted selection and stop its log stream; clear the mark if it returns.
    fn check_selection_present(&mut self) {
        let Some(r) = self.selected.clone() else {
            return;
        };
        if self.selection_present(&r) {
            self.viewer_notice = None;
            return;
        }
        self.viewer_notice = Some(SafeText::from(format!(
            "{} {} is no longer present",
            r.kind_label(),
            r.name()
        )));
        if self.log_state == LogState::Streaming {
            self.log_session += 1;
            self.commands.push(Command::StopLogs);
            self.log_state = LogState::Ended(SafeText::from("pod no longer present"));
        }
    }

    /// Start logs for a selected pod that has started since it was selected.
    fn ensure_log_stream(&mut self) {
        let started = self
            .selected_pod()
            .is_some_and(crate::model::Pod::has_started);
        if self.is_live() && started && self.log_state == LogState::Idle {
            self.restart_logs();
        }
    }

    fn request_yaml_if_needed(&mut self) {
        if !self.is_live() || self.current_tab() != Some(ViewerTab::Yaml) {
            return;
        }
        let Some(target) = self.selected.clone() else {
            return;
        };
        if self.yaml.as_ref().is_some_and(|(t, _)| *t == target) {
            return;
        }
        self.yaml = Some((target.clone(), YamlState::Loading));
        self.commands.push(Command::FetchYaml {
            generation: self.generation,
            target,
        });
    }

    /// Rows in a list before filtering.
    pub fn total_rows(&self, kind: ResourceKind) -> usize {
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

    /// Names in display order, used for filtering.
    fn row_names(&self, kind: ResourceKind) -> Vec<&str> {
        match kind {
            ResourceKind::Pods => self.snapshot.pods.iter().map(|p| p.name.as_str()).collect(),
            ResourceKind::Images => self.images.iter().map(|i| i.raw.as_str()).collect(),
            ResourceKind::Services => self
                .snapshot
                .services
                .iter()
                .map(|s| s.name.as_str())
                .collect(),
            ResourceKind::Storage => self.storage_rows().map(|s| s.name.as_str()).collect(),
        }
    }

    /// Display-order indices of the rows that pass the list's filter.
    pub fn visible_rows(&self, kind: ResourceKind) -> Vec<usize> {
        let filter = self.filters[kind.index()].to_lowercase();
        if filter.is_empty() {
            return (0..self.total_rows(kind)).collect();
        }
        self.row_names(kind)
            .iter()
            .enumerate()
            .filter(|(_, name)| name.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect()
    }

    /// Rows shown in a list (after filtering).
    pub fn row_count(&self, kind: ResourceKind) -> usize {
        self.visible_rows(kind).len()
    }

    /// The resource on visible row `idx` (after filtering).
    pub fn row_ref(&self, kind: ResourceKind, idx: usize) -> Option<ResourceRef> {
        let row = *self.visible_rows(kind).get(idx)?;
        self.display_ref(kind, row)
    }

    /// The resource at display-order index `row` (before filtering).
    pub fn display_ref(&self, kind: ResourceKind, row: usize) -> Option<ResourceRef> {
        match kind {
            ResourceKind::Pods => self
                .snapshot
                .pods
                .get(row)
                .map(|p| ResourceRef::Pod(p.name.clone())),
            ResourceKind::Images => self
                .images
                .get(row)
                .map(|i| ResourceRef::Image(i.raw.clone())),
            ResourceKind::Services => self
                .snapshot
                .services
                .get(row)
                .map(|s| ResourceRef::Service(s.name.clone())),
            ResourceKind::Storage => self.storage_rows().nth(row).map(|s| match s.kind {
                StorageKind::Pvc => ResourceRef::Pvc(s.name.clone()),
                StorageKind::Pv => ResourceRef::Pv(s.name.clone()),
            }),
        }
    }

    /// Visible row showing `r`, if any.
    fn find_row(&self, kind: ResourceKind, r: &ResourceRef) -> Option<usize> {
        self.visible_rows(kind)
            .iter()
            .position(|&row| self.display_ref(kind, row).as_ref() == Some(r))
    }

    pub fn filter(&self, kind: ResourceKind) -> &str {
        &self.filters[kind.index()]
    }

    /// Contexts offered by the `x` picker (from kubeconfig).
    pub fn set_contexts(&mut self, contexts: Vec<ContextInfo>) {
        self.contexts = contexts;
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
    /// Selecting the resource already shown keeps the viewer as it is.
    pub fn select(&mut self, r: ResourceRef) {
        if self.selected.as_ref() == Some(&r) {
            return;
        }
        self.selected = Some(r);
        self.tab = 0;
        self.container = 0;
        self.viewer_scroll = 0;
        self.viewer_notice = None;
        self.yaml = None;
        self.restart_logs();
    }

    /// Focus a pane (also used by mouse handling).
    /// Focusing a list leaves zoom, since lists are hidden while zoomed.
    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        if let Focus::List(kind) = focus {
            self.last_list = kind;
            self.zoom = false;
        }
    }

    fn toggle_zoom(&mut self) {
        self.zoom = !self.zoom;
        if self.zoom {
            self.focus = Focus::Viewer;
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
        if self.is_live() {
            self.restart_live_logs();
            return;
        }
        let lines = match (self.selected_pod(), self.current_container()) {
            (Some(pod), Some(container)) => {
                self.fake_logs.initial(pod, container.as_str(), LOG_TAIL)
            }
            _ => Vec::new(),
        };
        self.logs.extend(lines.into_iter().map(SafeText::from));
    }

    /// Replace the log stream with the selected pod's current container, or stop it.
    fn restart_live_logs(&mut self) {
        self.log_session += 1;
        let target = match (self.selected_pod(), self.current_container()) {
            (Some(pod), Some(container)) if pod.has_started() => {
                Some((pod.name.clone(), container.clone()))
            }
            _ => None,
        };
        match target {
            Some((pod, container)) => {
                self.commands.push(Command::StartLogs {
                    session: self.log_session,
                    pod,
                    container,
                });
                self.log_state = LogState::Streaming;
            }
            None => {
                if self.log_state == LogState::Streaming {
                    self.commands.push(Command::StopLogs);
                }
                self.log_state = LogState::Idle;
            }
        }
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.is_live() {
            if self.ticks.saturating_sub(self.last_refresh_tick) >= REFRESH_TICKS {
                self.request_refresh();
            }
            return;
        }
        if !self.ticks.is_multiple_of(TICKS_PER_LINE) {
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
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.picker.is_some() {
            self.on_picker_key(key);
            return;
        }
        if let Some(kind) = self.filter_editing {
            self.on_filter_key(kind, key);
            return;
        }
        // Only plain (or shifted) keys are bound; Ctrl/Alt/Super combinations are ignored.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return;
        }
        if key.code == KeyCode::Char('q') {
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
            KeyCode::Char('z') => self.toggle_zoom(),
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
                self.logs.set_follow(on, self.viewer_rows);
            }
            KeyCode::Char('c') => self.cycle_container(),
            KeyCode::Char('r') => self.request_refresh(),
            KeyCode::Char('/') => {
                if let Focus::List(kind) = self.focus {
                    self.filter_editing = Some(kind);
                }
            }
            KeyCode::Char('x') => self.open_context_picker(),
            KeyCode::Char('n') => self.open_namespace_picker(),
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

    /// True when the viewer shows a log stream (Logs tab of a pod that has started).
    pub fn shows_log_stream(&self) -> bool {
        self.current_tab() == Some(ViewerTab::Logs)
            && self
                .selected_pod()
                .is_some_and(crate::model::Pod::has_started)
    }

    pub fn scroll_viewer(&mut self, delta: isize) {
        if self.shows_log_stream() {
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
        if self.help || self.picker.is_some() {
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
            self.request_yaml_if_needed();
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
    fn zoom_focuses_viewer_and_list_keys_leave_it() {
        let mut a = app();
        a.update(key(KeyCode::Char('z')));
        assert_eq!(a.focus, Focus::Viewer);
        a.update(key(KeyCode::Enter));
        assert_eq!(
            a.selected,
            pod("api-7c9d8f-x2k4"),
            "enter does not act on a hidden list"
        );
        a.update(key(KeyCode::Char('3')));
        assert!(!a.zoom, "jumping to a list leaves zoom");
        assert_eq!(a.focus, Focus::List(ResourceKind::Services));
    }

    #[test]
    fn reselecting_same_row_keeps_viewer_state() {
        let mut a = app();
        a.update(key(KeyCode::Right));
        a.update(AppEvent::Tick);
        a.update(AppEvent::Tick);
        a.update(key(KeyCode::Enter));
        assert_eq!(a.current_tab(), Some(ViewerTab::Summary));
        assert_eq!(a.logs.len(), 35, "log stream is not restarted");
    }

    #[test]
    fn modified_keys_do_not_trigger_plain_actions() {
        let mut a = app();
        a.update(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )));
        a.update(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::ALT,
        )));
        assert!(!a.zoom && !a.quit);
        a.update(AppEvent::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert_eq!(a.focus, Focus::Viewer, "shift is allowed");
    }

    #[test]
    fn pending_pod_logs_tab_scrolls_viewer() {
        let mut a = app();
        for _ in 0..5 {
            a.update(key(KeyCode::Down));
        }
        a.update(key(KeyCode::Enter));
        a.set_focus(Focus::Viewer);
        a.update(key(KeyCode::Down));
        assert_eq!(a.viewer_scroll, 1);
    }

    #[test]
    fn slash_filters_focused_list() {
        let mut a = app();
        a.update(key(KeyCode::Char('/')));
        assert_eq!(a.filter_editing, Some(ResourceKind::Pods));
        for c in "BILL".chars() {
            a.update(key(KeyCode::Char(c)));
        }
        a.update(key(KeyCode::Enter));
        assert_eq!(a.filter_editing, None);
        assert_eq!(a.row_count(ResourceKind::Pods), 1);
        assert_eq!(a.row_ref(ResourceKind::Pods, 0), pod("billing-0"));
        assert_eq!(a.cursors[0].selected(), Some(0));
        a.update(key(KeyCode::Enter));
        assert_eq!(a.selected, pod("billing-0"));
    }

    #[test]
    fn filter_typing_does_not_trigger_keys() {
        let mut a = app();
        a.update(key(KeyCode::Char('/')));
        for c in "q3z".chars() {
            a.update(key(KeyCode::Char(c)));
        }
        assert!(!a.quit && !a.zoom);
        assert_eq!(a.filter(ResourceKind::Pods), "q3z");
        assert_eq!(a.row_count(ResourceKind::Pods), 0);
        a.update(key(KeyCode::Backspace));
        assert_eq!(a.filter(ResourceKind::Pods), "q3");
    }

    #[test]
    fn esc_clears_filter() {
        let mut a = app();
        a.update(key(KeyCode::Char('/')));
        a.update(key(KeyCode::Char('x')));
        a.update(key(KeyCode::Esc));
        assert_eq!(a.filter(ResourceKind::Pods), "");
        assert_eq!(a.row_count(ResourceKind::Pods), 6);
        assert_eq!(a.filter_editing, None);
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

    mod live {
        use super::*;
        use crate::cluster::config::ContextInfo;
        use crate::cluster::fetch::{FetchError, Lists};
        use crate::command::Command;
        use crate::state::PickerKind;

        fn lists(snap: &crate::model::ClusterSnapshot) -> Lists {
            Lists {
                pods: Ok(snap.pods.clone()),
                services: Ok(snap.services.clone()),
                storage: Ok(snap.storage.clone()),
                storage_partial: None,
                events: Ok(snap.events.clone()),
            }
        }

        fn data(ev: DataEvent) -> AppEvent {
            AppEvent::Data(ev)
        }

        fn live() -> App {
            App::new_live(
                LiveTarget {
                    context: "dev".into(),
                    namespace: "shop".into(),
                },
                false,
            )
        }

        /// A live app after its first successful refresh, with commands drained.
        fn loaded() -> App {
            let mut a = live();
            a.take_commands();
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&sample_snapshot())),
            }));
            a.take_commands();
            a
        }

        #[test]
        fn starts_loading_and_requests_refresh() {
            let mut a = live();
            assert!(
                a.panes
                    .iter()
                    .all(|p| matches!(p, PaneStatus::Loading { .. }))
            );
            assert_eq!(a.connection, Connection::Connecting);
            assert_eq!(a.selected, None);
            assert_eq!(a.snapshot.namespace.as_str(), "shop");
            assert_eq!(a.take_commands(), [Command::Refresh { generation: 0 }]);
            assert!(a.take_commands().is_empty(), "queue drains");
        }

        #[test]
        fn first_lists_select_first_pod_and_start_logs() {
            let mut a = live();
            a.take_commands();
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&sample_snapshot())),
            }));
            assert_eq!(a.selected, pod("api-7c9d8f-x2k4"));
            assert!(a.panes.iter().all(|p| *p == PaneStatus::Ready));
            assert_eq!(a.images.len(), 7);
            assert_eq!(a.logs.len(), 0, "no fake logs in live mode");
            assert_eq!(
                a.take_commands(),
                [Command::StartLogs {
                    session: 1,
                    pod: "api-7c9d8f-x2k4".into(),
                    container: "api".into()
                }]
            );
        }

        #[test]
        fn log_lines_for_current_session_only() {
            let mut a = loaded();
            a.update(data(DataEvent::LogLine {
                session: 0,
                line: "old".into(),
            }));
            a.update(data(DataEvent::LogLine {
                session: 1,
                line: "new".into(),
            }));
            assert_eq!(a.logs.visible(5).collect::<Vec<_>>(), ["new"]);
            a.update(data(DataEvent::LogEnded {
                session: 1,
                error: None,
            }));
            assert_eq!(a.log_state, LogState::Ended("stream closed".into()));
        }

        #[test]
        fn container_switch_restarts_stream() {
            let mut a = loaded();
            a.update(key(KeyCode::Char('c')));
            assert_eq!(
                a.take_commands(),
                [Command::StartLogs {
                    session: 2,
                    pod: "api-7c9d8f-x2k4".into(),
                    container: "envoy".into()
                }]
            );
        }

        #[test]
        fn selecting_service_stops_logs() {
            let mut a = loaded();
            a.update(AppEvent::Data(DataEvent::LogLine {
                session: 1,
                line: "x".into(),
            }));
            a.update(key(KeyCode::Char('3')));
            a.update(key(KeyCode::Enter));
            assert_eq!(a.take_commands(), [Command::StopLogs]);
            assert!(a.logs.is_empty());
            assert_eq!(a.log_state, LogState::Idle);
        }

        #[test]
        fn yaml_tab_requests_yaml_once_and_applies_matching_result() {
            let mut a = loaded();
            for _ in 0..3 {
                a.update(key(KeyCode::Right));
            }
            assert_eq!(a.current_tab(), Some(ViewerTab::Yaml));
            let target = ResourceRef::Pod("api-7c9d8f-x2k4".into());
            assert_eq!(
                a.take_commands(),
                [Command::FetchYaml {
                    generation: 0,
                    target: target.clone()
                }]
            );
            a.update(key(KeyCode::Left));
            a.update(key(KeyCode::Right));
            assert!(a.take_commands().is_empty(), "already loading");
            a.update(data(DataEvent::Yaml {
                generation: 0,
                target: ResourceRef::Pod("other".into()),
                result: Ok("kind: Other".into()),
            }));
            assert_eq!(a.yaml_state(), Some(&YamlState::Loading));
            a.update(data(DataEvent::Yaml {
                generation: 0,
                target,
                result: Ok("kind: Pod".into()),
            }));
            assert_eq!(a.yaml_state(), Some(&YamlState::Ready("kind: Pod".into())));
        }

        #[test]
        fn stale_generation_ignored() {
            let mut a = live();
            a.update(data(DataEvent::Lists {
                generation: 7,
                lists: Box::new(lists(&sample_snapshot())),
            }));
            assert!(a.snapshot.pods.is_empty());
            a.update(data(DataEvent::Version {
                generation: 7,
                result: Ok("v1".into()),
            }));
            assert_eq!(a.connection, Connection::Connecting);
        }

        #[test]
        fn selection_kept_by_name_after_refresh() {
            let mut a = loaded();
            for _ in 0..3 {
                a.update(key(KeyCode::Down));
            }
            let mut snap = sample_snapshot();
            snap.pods.remove(0);
            a.request_refresh();
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&snap)),
            }));
            let idx = a.cursors[ResourceKind::Pods.index()].selected().unwrap();
            assert_eq!(a.row_ref(ResourceKind::Pods, idx), pod("billing-0"));
        }

        #[test]
        fn selected_pod_disappears_sets_notice_and_stops_logs() {
            let mut a = loaded();
            let mut snap = sample_snapshot();
            snap.pods.remove(0);
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&snap)),
            }));
            assert_eq!(
                a.viewer_notice.as_ref().map(SafeText::as_str),
                Some("Pod api-7c9d8f-x2k4 is no longer present")
            );
            assert_eq!(a.take_commands(), [Command::StopLogs]);
            assert_eq!(a.log_state, LogState::Ended("pod no longer present".into()));
        }

        #[test]
        fn forbidden_pane_keeps_others() {
            let mut a = live();
            let mut l = lists(&sample_snapshot());
            let forbidden = FetchError::Forbidden {
                verb: "list",
                resource: "persistentvolumeclaims",
                scope: "namespace \"shop\"".into(),
                message: "no".into(),
            };
            l.storage = Err(forbidden.clone());
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(l),
            }));
            assert_eq!(
                a.panes[ResourceKind::Storage.index()],
                PaneStatus::Failed(forbidden)
            );
            assert_eq!(a.panes[ResourceKind::Pods.index()], PaneStatus::Ready);
        }

        #[test]
        fn storage_partial_is_recorded() {
            let mut a = live();
            let mut l = lists(&sample_snapshot());
            l.storage_partial = Some(FetchError::Timeout);
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(l),
            }));
            assert_eq!(
                a.panes[ResourceKind::Storage.index()],
                PaneStatus::Partial(FetchError::Timeout)
            );
        }

        #[test]
        fn failed_refresh_keeps_last_items_as_stale() {
            let mut a = loaded();
            let mut l = lists(&sample_snapshot());
            l.pods = Err(FetchError::Timeout);
            a.request_refresh();
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(l),
            }));
            assert_eq!(a.snapshot.pods.len(), 6);
            assert_eq!(
                a.panes[ResourceKind::Pods.index()],
                PaneStatus::Failed(FetchError::Timeout)
            );
            assert!(a.is_stale(ResourceKind::Pods));
            assert!(a.is_stale(ResourceKind::Images), "images derive from pods");
        }

        #[test]
        fn version_sets_connection() {
            let mut a = live();
            a.update(data(DataEvent::Version {
                generation: 0,
                result: Ok("v1.36.4".into()),
            }));
            assert_eq!(
                a.connection,
                Connection::Connected {
                    version: "v1.36.4".into()
                }
            );
            a.update(data(DataEvent::Version {
                generation: 0,
                result: Err(FetchError::Unauthorized("token expired".into())),
            }));
            assert_eq!(
                a.connection,
                Connection::Disconnected {
                    error: "Unauthorized (401): token expired".into()
                }
            );
        }

        #[test]
        fn auto_refresh_every_40_ticks_without_overlap() {
            let mut a = loaded();
            for _ in 0..39 {
                a.update(AppEvent::Tick);
            }
            assert!(a.take_commands().is_empty());
            a.update(AppEvent::Tick);
            assert_eq!(a.take_commands(), [Command::Refresh { generation: 0 }]);
            for _ in 0..80 {
                a.update(AppEvent::Tick);
            }
            assert!(
                a.take_commands().is_empty(),
                "no new refresh while one is in flight"
            );
        }

        #[test]
        fn r_requests_refresh_when_idle() {
            let mut a = loaded();
            a.update(key(KeyCode::Char('r')));
            assert_eq!(a.take_commands(), [Command::Refresh { generation: 0 }]);
            a.update(key(KeyCode::Char('r')));
            assert!(a.take_commands().is_empty());
        }

        #[test]
        fn pending_pod_starts_logs_once_running() {
            let mut a = loaded();
            for _ in 0..5 {
                a.update(key(KeyCode::Down));
            }
            a.update(key(KeyCode::Enter));
            assert_eq!(a.take_commands(), [Command::StopLogs]);
            let mut snap = sample_snapshot();
            snap.pods[5].status = crate::model::PodStatus::Running;
            a.request_refresh();
            a.take_commands();
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&snap)),
            }));
            assert_eq!(
                a.take_commands(),
                [Command::StartLogs {
                    session: 3,
                    pod: "ledger-sync-5f7d-h2c".into(),
                    container: "ledger-sync".into()
                }]
            );
        }

        fn contexts() -> Vec<ContextInfo> {
            let info = |name: &str, ns: Option<&str>| ContextInfo {
                name: name.into(),
                cluster: name.into(),
                server_host: "k8s.example.test".into(),
                namespace: ns.map(SafeText::from),
                hint: "rancher",
            };
            vec![
                info("dev", Some("shop")),
                info("dev-west", None),
                info("prod", Some("ops")),
            ]
        }

        fn typed(a: &mut App, text: &str) {
            for c in text.chars() {
                a.update(key(KeyCode::Char(c)));
            }
        }

        #[test]
        fn x_opens_context_picker_marking_current() {
            let mut a = loaded();
            a.set_contexts(contexts());
            a.update(key(KeyCode::Char('x')));
            let picker = a.picker.as_ref().expect("picker open");
            assert_eq!(picker.kind, PickerKind::Context);
            assert_eq!(picker.total(), 3);
            assert_eq!(
                picker.chosen().unwrap().as_str(),
                "dev",
                "cursor starts on current"
            );
            assert!(picker.matches()[0].current);
            assert_eq!(
                picker.matches()[0].detail.as_str(),
                "rancher · k8s.example.test"
            );
        }

        #[test]
        fn typing_in_picker_does_not_quit_or_navigate() {
            let mut a = loaded();
            a.set_contexts(contexts());
            a.update(key(KeyCode::Char('x')));
            typed(&mut a, "q3");
            assert!(!a.quit);
            assert_eq!(a.focus, Focus::List(ResourceKind::Pods));
            assert_eq!(a.picker.as_ref().unwrap().query, "q3");
            a.update(key(KeyCode::Esc));
            assert!(a.picker.is_none());
        }

        #[test]
        fn enter_switches_context_and_bumps_generation() {
            let mut a = loaded();
            a.set_contexts(contexts());
            a.update(key(KeyCode::Char('x')));
            typed(&mut a, "west");
            a.update(key(KeyCode::Enter));
            assert_eq!(
                a.take_commands(),
                [
                    Command::StopLogs,
                    Command::SwitchTarget {
                        context: "dev-west".into(),
                        namespace: "default".into()
                    },
                    Command::Refresh { generation: 1 },
                ]
            );
            assert!(a.picker.is_none());
            assert_eq!(a.snapshot.context.as_str(), "dev-west");
            assert_eq!(a.snapshot.namespace.as_str(), "default");
            assert!(a.snapshot.pods.is_empty() && a.selected.is_none() && a.logs.is_empty());
            assert_eq!(a.connection, Connection::Connecting);
            assert!(matches!(a.panes[0], PaneStatus::Loading { .. }));
            a.update(data(DataEvent::Lists {
                generation: 0,
                lists: Box::new(lists(&sample_snapshot())),
            }));
            assert!(
                a.snapshot.pods.is_empty(),
                "late answer for the old context is dropped"
            );
        }

        #[test]
        fn n_lists_namespaces_and_switches_namespace() {
            let mut a = loaded();
            a.update(key(KeyCode::Char('n')));
            assert_eq!(a.take_commands(), [Command::FetchNamespaces]);
            assert!(a.picker.as_ref().unwrap().loading);
            a.update(data(DataEvent::Namespaces(Ok(vec![
                "kube-system".into(),
                "shop".into(),
                "team-a".into(),
            ]))));
            assert_eq!(
                a.picker.as_ref().unwrap().chosen().unwrap().as_str(),
                "shop"
            );
            typed(&mut a, "team");
            a.update(key(KeyCode::Enter));
            let cmds = a.take_commands();
            assert!(
                cmds.contains(&Command::SwitchTarget {
                    context: "dev".into(),
                    namespace: "team-a".into()
                }),
                "{cmds:?}"
            );
            assert_eq!(a.snapshot.namespace.as_str(), "team-a");
        }

        #[test]
        fn namespace_picker_free_text_when_forbidden() {
            let mut a = loaded();
            a.update(key(KeyCode::Char('n')));
            a.take_commands();
            a.update(data(DataEvent::Namespaces(Err(FetchError::Timeout))));
            typed(&mut a, "secret-team");
            a.update(key(KeyCode::Enter));
            assert!(a.take_commands().contains(&Command::SwitchTarget {
                context: "dev".into(),
                namespace: "secret-team".into()
            }));
        }

        #[test]
        fn pickers_unavailable_in_sample_mode() {
            let mut a = app();
            a.update(key(KeyCode::Char('x')));
            a.update(key(KeyCode::Char('n')));
            assert!(a.picker.is_none());
            assert!(a.take_commands().is_empty());
        }

        #[test]
        fn sample_mode_queues_no_commands() {
            let mut a = app();
            a.update(key(KeyCode::Down));
            a.update(key(KeyCode::Enter));
            a.update(key(KeyCode::Char('r')));
            for _ in 0..50 {
                a.update(AppEvent::Tick);
            }
            assert!(a.take_commands().is_empty());
            assert_eq!(a.connection, Connection::Sample);
        }
    }
}
