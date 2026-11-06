//! The imperative shell: terminal input, ticks, signals and API calls around
//! the pure `App` core.

use std::time::Duration;

use futures::StreamExt;
use kube::Client;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::app::{App, AppEvent, DataEvent, LOG_TAIL};
use crate::cluster::config::{Kubeconfig, Target, client_for};
use crate::cluster::convert::Timestamp;
use crate::cluster::fetch::{self, FetchError};
use crate::command::Command;
use crate::text::SafeText;
use crate::ui;

/// Interval between `AppEvent::Tick`s.
pub const TICK: Duration = Duration::from_millis(250);
/// Bounded so a chatty log stream waits instead of growing memory.
const DATA_CHANNEL: usize = 4096;
/// API results applied per frame before redrawing.
const MAX_EVENTS_PER_FRAME: usize = 2000;

pub fn map_terminal_event(ev: Event) -> Option<AppEvent> {
    match ev {
        Event::Key(key) => Some(AppEvent::Key(key)),
        Event::Mouse(mouse) => Some(AppEvent::Mouse(mouse)),
        Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
        _ => None,
    }
}

/// Runs `Command`s against the cluster and reports results on `tx`.
/// Without a client (sample mode) commands are ignored.
pub struct Executor {
    client: Option<Client>,
    namespace: String,
    tx: mpsc::Sender<DataEvent>,
    log_task: Option<JoinHandle<()>>,
    /// Needed to build clients for other contexts (`SwitchTarget`).
    kubeconfig: Option<Kubeconfig>,
    /// Why the last target switch produced no client; reported on refresh.
    switch_error: Option<FetchError>,
}

impl Executor {
    pub fn new(client: Option<Client>, namespace: String, tx: mpsc::Sender<DataEvent>) -> Self {
        Executor {
            client,
            namespace,
            tx,
            log_task: None,
            kubeconfig: None,
            switch_error: None,
        }
    }

    pub fn with_kubeconfig(mut self, kubeconfig: Kubeconfig) -> Self {
        self.kubeconfig = Some(kubeconfig);
        self
    }

    pub async fn run(&mut self, command: Command) {
        if let Command::SwitchTarget { context, namespace } = command {
            self.switch_target(context.to_string(), namespace.to_string())
                .await;
            return;
        }
        let Some(client) = self.client.clone() else {
            self.report_switch_error(&command).await;
            return;
        };
        let tx = self.tx.clone();
        let ns = self.namespace.clone();
        match command {
            Command::SwitchTarget { .. } => unreachable!("handled above"),
            Command::FetchNamespaces => {
                tokio::spawn(async move {
                    let result = fetch::namespaces(&client).await;
                    let _ = tx.send(DataEvent::Namespaces(result)).await;
                });
            }
            Command::Refresh { generation } => {
                let version_client = client.clone();
                let version_tx = tx.clone();
                tokio::spawn(async move {
                    let lists = fetch::fetch_lists(&client, &ns, Timestamp::now()).await;
                    let _ = tx
                        .send(DataEvent::Lists {
                            generation,
                            lists: Box::new(lists),
                        })
                        .await;
                });
                tokio::spawn(async move {
                    let result = fetch::server_version(&version_client).await;
                    let _ = version_tx
                        .send(DataEvent::Version { generation, result })
                        .await;
                });
            }
            Command::StartLogs {
                session,
                pod,
                container,
            } => {
                self.stop_logs();
                let tail = i64::try_from(LOG_TAIL).unwrap_or(i64::MAX);
                self.log_task = Some(tokio::spawn(async move {
                    let lines =
                        fetch::log_lines(client, ns, pod.to_string(), container.to_string(), tail);
                    futures::pin_mut!(lines);
                    while let Some(item) = lines.next().await {
                        let event = match item {
                            Ok(line) => DataEvent::LogLine { session, line },
                            Err(e) => {
                                let _ = tx
                                    .send(DataEvent::LogEnded {
                                        session,
                                        error: Some(e),
                                    })
                                    .await;
                                return;
                            }
                        };
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    let _ = tx
                        .send(DataEvent::LogEnded {
                            session,
                            error: None,
                        })
                        .await;
                }));
            }
            Command::StopLogs => self.stop_logs(),
            Command::FetchYaml { generation, target } => {
                tokio::spawn(async move {
                    let result = fetch::fetch_yaml(&client, &ns, &target).await;
                    let _ = tx
                        .send(DataEvent::Yaml {
                            generation,
                            target,
                            result,
                        })
                        .await;
                });
            }
        }
    }

    async fn switch_target(&mut self, context: String, namespace: String) {
        self.stop_logs();
        self.namespace = namespace.clone();
        self.switch_error = None;
        let Some(kubeconfig) = self.kubeconfig.clone() else {
            return;
        };
        let target = Target {
            kubeconfig,
            context,
            namespace,
        };
        match client_for(&target).await {
            Ok(client) => self.client = Some(client),
            Err(e) => {
                self.client = None;
                self.switch_error = Some(FetchError::Unreachable(SafeText::from(e.to_string())));
            }
        }
    }

    /// After a failed switch, answer refreshes with the error so panes do not wait forever.
    async fn report_switch_error(&self, command: &Command) {
        let (Some(error), Command::Refresh { generation }) = (&self.switch_error, command) else {
            return;
        };
        let lists = fetch::Lists {
            pods: Err(error.clone()),
            services: Err(error.clone()),
            storage: Err(error.clone()),
            storage_partial: None,
            events: Err(error.clone()),
        };
        let _ = self
            .tx
            .send(DataEvent::Version {
                generation: *generation,
                result: Err(error.clone()),
            })
            .await;
        let _ = self
            .tx
            .send(DataEvent::Lists {
                generation: *generation,
                lists: Box::new(lists),
            })
            .await;
    }

    fn stop_logs(&mut self) {
        if let Some(task) = self.log_task.take() {
            task.abort();
        }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.stop_logs();
    }
}

/// Resolves on SIGTERM, SIGHUP or SIGINT so the terminal can be restored.
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
        signal(SignalKind::interrupt()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = hup.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Drive `app` until it quits: draw, wait for the next input, tick, API result
/// or signal, then execute the commands it queued.
pub async fn run(
    terminal: &mut DefaultTerminal,
    mut app: App,
    client: Option<Client>,
    namespace: String,
    kubeconfig: Option<Kubeconfig>,
) -> color_eyre::Result<()> {
    let (data_tx, mut data_rx) = mpsc::channel(DATA_CHANNEL);
    let mut executor = Executor::new(client, namespace, data_tx);
    if let Some(kc) = kubeconfig {
        executor = executor.with_kubeconfig(kc);
    }

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    // crossterm's `read` blocks, so it gets its own thread. It ends with the process.
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    while !app.quit {
        for command in app.take_commands() {
            executor.run(command).await;
        }
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        tokio::select! {
            Some(ev) = input_rx.recv() => {
                if let Some(ev) = map_terminal_event(ev) {
                    app.update(ev);
                }
            }
            _ = ticker.tick() => app.update(AppEvent::Tick),
            Some(data) = data_rx.recv() => {
                app.update(AppEvent::Data(data));
                for _ in 0..MAX_EVENTS_PER_FRAME {
                    match data_rx.try_recv() {
                        Ok(more) => app.update(AppEvent::Data(more)),
                        Err(_) => break,
                    }
                }
            }
            () = &mut shutdown => app.quit = true,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::testing::{NS, healthy, mock};
    use crate::model::ResourceRef;

    fn executor() -> (Executor, mpsc::Receiver<DataEvent>) {
        let (tx, rx) = mpsc::channel(64);
        (Executor::new(Some(mock(healthy)), NS.to_string(), tx), rx)
    }

    async fn next(rx: &mut mpsc::Receiver<DataEvent>) -> DataEvent {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("event arrives")
            .expect("channel open")
    }

    #[tokio::test]
    async fn refresh_sends_lists_and_version() {
        let (mut ex, mut rx) = executor();
        ex.run(Command::Refresh { generation: 3 }).await;
        let mut got_lists = false;
        let mut got_version = false;
        for _ in 0..2 {
            match next(&mut rx).await {
                DataEvent::Lists {
                    generation: 3,
                    lists,
                } => {
                    got_lists = true;
                    assert_eq!(lists.pods.unwrap().len(), 7);
                }
                DataEvent::Version {
                    generation: 3,
                    result,
                } => {
                    got_version = true;
                    assert_eq!(result.unwrap().as_str(), "v1.36.4+rke2r1");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(got_lists && got_version);
    }

    #[tokio::test]
    async fn logs_stream_then_end() {
        let (mut ex, mut rx) = executor();
        ex.run(Command::StartLogs {
            session: 9,
            pod: "api-1".into(),
            container: "api".into(),
        })
        .await;
        let mut lines = Vec::new();
        loop {
            match next(&mut rx).await {
                DataEvent::LogLine { session: 9, line } => lines.push(line.to_string()),
                DataEvent::LogEnded { session: 9, error } => {
                    assert_eq!(error, None);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn stop_logs_aborts_stream() {
        let (mut ex, _rx) = executor();
        ex.run(Command::StartLogs {
            session: 1,
            pod: "api-1".into(),
            container: "api".into(),
        })
        .await;
        assert!(ex.log_task.is_some());
        ex.run(Command::StopLogs).await;
        assert!(ex.log_task.is_none());
    }

    #[tokio::test]
    async fn yaml_sent_for_target() {
        let (mut ex, mut rx) = executor();
        let target = ResourceRef::Pod("api-1".into());
        ex.run(Command::FetchYaml {
            generation: 0,
            target: target.clone(),
        })
        .await;
        match next(&mut rx).await {
            DataEvent::Yaml {
                generation: 0,
                target: t,
                result,
            } => {
                assert_eq!(t, target);
                assert!(result.unwrap().contains("name: api-1"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    fn fixture_kubeconfig() -> kube::config::Kubeconfig {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/kubeconfig-b.yaml");
        kube::config::Kubeconfig::read_from(path).unwrap()
    }

    #[tokio::test]
    async fn namespaces_listed_for_picker() {
        let (mut ex, mut rx) = executor();
        ex.run(Command::FetchNamespaces).await;
        match next(&mut rx).await {
            DataEvent::Namespaces(Ok(names)) => assert_eq!(names.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn switch_target_rebuilds_client_for_new_namespace() {
        let (tx, _rx) = mpsc::channel(4);
        let mut ex = Executor::new(None, "a".into(), tx).with_kubeconfig(fixture_kubeconfig());
        ex.run(Command::SwitchTarget {
            context: "kind-local".into(),
            namespace: "team-b".into(),
        })
        .await;
        assert_eq!(ex.namespace, "team-b");
        assert_eq!(
            ex.client.as_ref().map(Client::default_namespace),
            Some("team-b")
        );
    }

    #[tokio::test]
    async fn failed_switch_reports_error_on_refresh() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut ex = Executor::new(None, "a".into(), tx).with_kubeconfig(fixture_kubeconfig());
        ex.run(Command::SwitchTarget {
            context: "nope".into(),
            namespace: "x".into(),
        })
        .await;
        ex.run(Command::Refresh { generation: 2 }).await;
        let mut saw_version_error = false;
        let mut saw_list_error = false;
        for _ in 0..2 {
            match next(&mut rx).await {
                DataEvent::Version {
                    generation: 2,
                    result: Err(_),
                } => saw_version_error = true,
                DataEvent::Lists {
                    generation: 2,
                    lists,
                } => saw_list_error = lists.pods.is_err() && lists.storage.is_err(),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(saw_version_error && saw_list_error);
    }

    #[tokio::test]
    async fn sample_mode_executor_ignores_commands() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut ex = Executor::new(None, NS.to_string(), tx);
        ex.run(Command::Refresh { generation: 0 }).await;
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn terminal_events_map_to_app_events() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            map_terminal_event(Event::Key(key)),
            Some(AppEvent::Key(key))
        );
        assert_eq!(
            map_terminal_event(Event::Resize(80, 24)),
            Some(AppEvent::Resize(80, 24))
        );
        assert_eq!(map_terminal_event(Event::FocusGained), None);
    }
}
