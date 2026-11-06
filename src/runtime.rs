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
use crate::cluster::convert::Timestamp;
use crate::cluster::fetch;
use crate::command::Command;
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
}

impl Executor {
    pub fn new(client: Option<Client>, namespace: String, tx: mpsc::Sender<DataEvent>) -> Self {
        Executor {
            client,
            namespace,
            tx,
            log_task: None,
        }
    }

    pub fn run(&mut self, command: Command) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.tx.clone();
        let ns = self.namespace.clone();
        match command {
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
) -> color_eyre::Result<()> {
    let (data_tx, mut data_rx) = mpsc::channel(DATA_CHANNEL);
    let mut executor = Executor::new(client, namespace, data_tx);

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
            executor.run(command);
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
        ex.run(Command::Refresh { generation: 3 });
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
        });
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
        });
        assert!(ex.log_task.is_some());
        ex.run(Command::StopLogs);
        assert!(ex.log_task.is_none());
    }

    #[tokio::test]
    async fn yaml_sent_for_target() {
        let (mut ex, mut rx) = executor();
        let target = ResourceRef::Pod("api-1".into());
        ex.run(Command::FetchYaml {
            generation: 0,
            target: target.clone(),
        });
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

    #[tokio::test]
    async fn sample_mode_executor_ignores_commands() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut ex = Executor::new(None, NS.to_string(), tx);
        ex.run(Command::Refresh { generation: 0 });
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
