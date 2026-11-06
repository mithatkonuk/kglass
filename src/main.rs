//! `kglass` binary: parse flags, set up the terminal, pump events into `App`.

use std::io::stdout;
use std::time::{Duration, Instant};

use clap::Parser;
use color_eyre::Result;
use kglass::app::{App, AppEvent};
use kglass::cli::Args;
use kglass::fake::sample_snapshot;
use kglass::ui;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;

/// Interval between `AppEvent::Tick`s (drives simulated log streaming).
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    let options = args.to_options(std::env::var("NO_COLOR").ok().as_deref());
    let app = App::new(sample_snapshot(), options);

    // `ratatui::init` enters raw mode and the alternate screen, and installs a
    // panic hook that restores both. Wrap it so mouse reporting is turned off too.
    let mut terminal = ratatui::init();
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        restore_terminal(info);
    }));

    let result = execute!(stdout(), EnableMouseCapture)
        .map_err(Into::into)
        .and_then(|()| run(&mut terminal, app));

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    let mut last_tick = Instant::now();
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.update(AppEvent::Key(key)),
                Event::Mouse(mouse) => app.update(AppEvent::Mouse(mouse)),
                Event::Resize(w, h) => app.update(AppEvent::Resize(w, h)),
                _ => {}
            }
        }
        if last_tick.elapsed() >= TICK {
            app.update(AppEvent::Tick);
            last_tick = Instant::now();
        }
    }
    Ok(())
}
