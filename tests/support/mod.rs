//! Shared helpers for rendering and interaction tests.
#![allow(dead_code)] // each test binary uses a different subset

use kglass::app::{App, AppEvent, AppOptions};
use kglass::fake::sample_snapshot;
use kglass::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use unicode_width::UnicodeWidthStr;

pub fn app() -> App {
    App::new(sample_snapshot(), AppOptions::default())
}

pub fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub fn click(column: u16, row: u16) -> AppEvent {
    AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

pub fn wheel(column: u16, row: u16, down: bool) -> AppEvent {
    AppEvent::Mouse(MouseEvent {
        kind: if down {
            MouseEventKind::ScrollDown
        } else {
            MouseEventKind::ScrollUp
        },
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

pub fn render_to_buffer(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw succeeds");
    terminal.backend().buffer().clone()
}

/// Buffer rows joined by newlines. Cells hidden behind wide characters are skipped
/// so that text like `日本語` reads as written.
pub fn render_to_string(app: &mut App, w: u16, h: u16) -> String {
    let buf = render_to_buffer(app, w, h);
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut skip = 0;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            skip = sym.width().saturating_sub(1);
            out.push_str(sym);
        }
        out.push('\n');
    }
    out
}

/// A live app that has not received any data yet.
pub fn live_app() -> App {
    App::new_live(
        kglass::app::LiveTarget {
            context: "dev-01".into(),
            namespace: "product-test".into(),
        },
        false,
    )
}

/// Lists built from the sample snapshot, as the API would return them.
pub fn sample_lists() -> kglass::cluster::fetch::Lists {
    let snap = sample_snapshot();
    kglass::cluster::fetch::Lists {
        pods: Ok(snap.pods),
        services: Ok(snap.services),
        storage: Ok(snap.storage),
        storage_partial: None,
        events: Ok(snap.events),
    }
}

pub fn lists_event(lists: kglass::cluster::fetch::Lists) -> AppEvent {
    AppEvent::Data(kglass::app::DataEvent::Lists {
        generation: 0,
        lists: Box::new(lists),
    })
}
