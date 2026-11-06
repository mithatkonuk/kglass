//! Diagnostic: fetch every list once from the current kubeconfig target and print
//! counts and timings. Read-only. Run with `cargo run --example probe`
//! (`-- --all` for all namespaces, `-- --frame` to render one screen).

use std::path::PathBuf;
use std::time::Instant;

use kglass::cluster::config::{client_for, load_kubeconfig, resolve};
use kglass::cluster::convert::Timestamp;
use kglass::cluster::fetch;

#[tokio::main]
async fn main() {
    let env = std::env::var("KUBECONFIG").ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let kc = load_kubeconfig(None, env.as_deref(), home.as_deref()).expect("kubeconfig");
    let target = resolve(kc, None, None).expect("target");
    println!("target: {target:?}");
    let client = client_for(&target).await.expect("client");
    let scope = if std::env::args().any(|a| a == "--all") {
        fetch::Scope::All
    } else {
        fetch::Scope::Namespace(target.namespace.clone())
    };
    println!("scope: {scope:?}");
    let t = Instant::now();
    println!(
        "version: {:?} ({:?})",
        fetch::server_version(&client).await,
        t.elapsed()
    );
    let t = Instant::now();
    let lists = fetch::fetch_lists(&client, &scope, Timestamp::now()).await;
    println!("lists took {:?}", t.elapsed());
    let count = |r: Result<usize, fetch::FetchError>| match r {
        Ok(n) => format!("{n}"),
        Err(e) => format!("ERR {e}"),
    };
    println!(
        "pods:     {}",
        count(lists.pods.as_ref().map(Vec::len).map_err(Clone::clone))
    );
    println!(
        "services: {}",
        count(lists.services.as_ref().map(Vec::len).map_err(Clone::clone))
    );
    println!(
        "storage:  {} partial={:?}",
        count(lists.storage.as_ref().map(Vec::len).map_err(Clone::clone)),
        lists.storage_partial
    );
    println!(
        "events:   {}",
        count(lists.events.as_ref().map(Vec::len).map_err(Clone::clone))
    );
    if let Ok(pods) = &lists.pods {
        let namespaces: std::collections::BTreeSet<&str> =
            pods.iter().map(|p| p.namespace.as_str()).collect();
        println!("pod namespaces: {}", namespaces.len());
        println!(
            "first pod key: {}",
            pods.first().map_or("-", |p| p.name.as_str())
        );
    }
    if std::env::args().any(|a| a == "--frame") {
        println!("{}", render_live_frame(client, &target).await);
    }
}

/// Render one frame of the live app after the first refresh (and a few log lines).
pub async fn render_live_frame(
    client: kube::Client,
    target: &kglass::cluster::config::Target,
) -> String {
    use kglass::app::{App, AppEvent, LiveTarget};
    use kglass::runtime::Executor;
    use ratatui::{Terminal, backend::TestBackend};

    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    let mut executor = Executor::new(
        Some(client),
        kglass::cluster::fetch::Scope::Namespace(target.namespace.clone()),
        tx,
    );
    let mut app = App::new_live(
        LiveTarget {
            context: target.context.as_str().into(),
            namespace: target.namespace.as_str().into(),
            all_namespaces: false,
        },
        true,
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(4);
    while Instant::now() < deadline {
        for c in app.take_commands() {
            executor.run(c).await;
        }
        if let Ok(Some(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            app.update(AppEvent::Data(ev));
        }
    }
    let mut term = Terminal::new(TestBackend::new(160, 48)).unwrap();
    term.draw(|f| kglass::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
