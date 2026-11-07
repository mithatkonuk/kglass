//! `kglass` binary: parse flags, pick the cluster like kubectl, run the TUI.

use std::io::stdout;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use kglass::app::{App, LiveTarget};
use kglass::cli::Args;
use kglass::cluster::config::{client_for, contexts, load_kubeconfig, resolve};
use kglass::cluster::fetch::Scope;
use kglass::fake::sample_snapshot;
use kglass::oci::Engine;
use kglass::runtime;
use kglass::text::SafeText;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = color_eyre::install() {
        eprintln!("kglass: {e}");
    }
    let args = Args::parse();
    if let Err(message) = args.check() {
        eprintln!("kglass: {message}");
        return ExitCode::from(2);
    }
    let options = args.to_options(std::env::var("NO_COLOR").ok().as_deref());

    let (app, client, scope, kubeconfig, oci) = if args.demo {
        (
            App::new(sample_snapshot(), options),
            None,
            Scope::Namespace(String::new()),
            None,
            None,
        )
    } else if let Some(command) = &args.oci {
        // Local containers through a Docker-compatible CLI; -n filters by compose project.
        let project = args.namespace.clone();
        let mut app = App::new_live(
            LiveTarget {
                context: SafeText::from(format!("oci: {command}")),
                namespace: SafeText::from(project.as_deref().unwrap_or("all")),
                all_namespaces: project.is_none(),
            },
            options.plain,
        );
        app.set_local_engine();
        let scope = project.map_or(Scope::All, Scope::Namespace);
        (app, None, scope, None, Some(Engine::new(command.clone())))
    } else {
        let env = std::env::var("KUBECONFIG").ok();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let target =
            match load_kubeconfig(args.kubeconfig.as_deref(), env.as_deref(), home.as_deref())
                .and_then(|kc| resolve(kc, args.context.as_deref(), args.namespace.as_deref()))
            {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("kglass: {e}\nRun `kglass --demo` to try it with sample data.");
                    return ExitCode::FAILURE;
                }
            };
        let client = match client_for(&target).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("kglass: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mut app = App::new_live(
            LiveTarget {
                context: SafeText::from(target.context.as_str()),
                namespace: SafeText::from(target.namespace.as_str()),
                all_namespaces: args.all_namespaces,
            },
            options.plain,
        );
        app.set_contexts(contexts(&target.kubeconfig));
        let scope = if args.all_namespaces {
            Scope::All
        } else {
            Scope::Namespace(target.namespace.clone())
        };
        (app, Some(client), scope, Some(target.kubeconfig), None)
    };

    // `ratatui::init` enters raw mode and the alternate screen and installs a panic
    // hook that restores both. Wrap it so mouse reporting is turned off too.
    let mut terminal = ratatui::init();
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Background tasks are guarded and report failures on the panes; restoring
        // the terminal for them would break the screen of a still-running UI.
        if std::thread::current().name() != Some("main") {
            return;
        }
        let _ = execute!(stdout(), DisableMouseCapture);
        restore_terminal(info);
    }));

    let result = match execute!(stdout(), EnableMouseCapture) {
        Ok(()) => runtime::run(&mut terminal, app, client, scope, kubeconfig, oci).await,
        Err(e) => Err(e.into()),
    };

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kglass: {e}");
            ExitCode::FAILURE
        }
    }
}
