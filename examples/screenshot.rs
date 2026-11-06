//! Render the `--demo` screen to `docs/screenshot.svg` for the README.
//! Run with `cargo run --example screenshot`. Uses sample data only.

use std::fmt::Write as _;

use kglass::app::{App, AppOptions};
use kglass::fake::sample_snapshot;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

const COLS: u16 = 160;
const ROWS: u16 = 48;
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 17.0;
const BACKGROUND: &str = "#0d1117";
const FOREGROUND: &str = "#c9d1d9";

fn hex(color: Color, default: &str) -> String {
    match color {
        Color::Reset => default.to_string(),
        Color::Black => "#0d1117".into(),
        Color::Red => "#f85149".into(),
        Color::Green => "#3fb950".into(),
        Color::Yellow => "#d29922".into(),
        Color::Blue => "#1f6feb".into(),
        Color::Magenta => "#bc8cff".into(),
        Color::Cyan => "#39c5cf".into(),
        Color::Gray | Color::White => "#e6edf3".into(),
        Color::DarkGray => "#6e7681".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(237) => "#3a3a3a".into(),
        _ => default.to_string(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn svg(buf: &Buffer) -> String {
    let (w, h) = (f32::from(COLS) * CELL_W, f32::from(ROWS) * CELL_H);
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="ui-monospace, SFMono-Regular, Menlo, monospace" font-size="14">"#
    );
    let _ = write!(
        out,
        r#"<rect width="100%" height="100%" fill="{BACKGROUND}"/>"#
    );
    for y in 0..ROWS {
        let mut skip = 0;
        for x in 0..COLS {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let cell = &buf[(x, y)];
            let symbol = cell.symbol();
            skip = symbol.width().saturating_sub(1);
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (fg, bg) = if reversed {
                (cell.bg, cell.fg)
            } else {
                (cell.fg, cell.bg)
            };
            let (px, py) = (f32::from(x) * CELL_W, f32::from(y) * CELL_H);
            if bg != Color::Reset || reversed {
                let width = CELL_W * symbol.width().max(1) as f32;
                let _ = write!(
                    out,
                    r#"<rect x="{px}" y="{py}" width="{width}" height="{CELL_H}" fill="{}"/>"#,
                    hex(bg, FOREGROUND)
                );
            }
            if symbol.trim().is_empty() {
                continue;
            }
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "bold"
            } else {
                "normal"
            };
            let decoration = if cell.modifier.contains(Modifier::UNDERLINED) {
                r#" text-decoration="underline""#
            } else {
                ""
            };
            let _ = write!(
                out,
                r#"<text x="{px}" y="{}" fill="{}" font-weight="{weight}"{decoration}>{}</text>"#,
                py + CELL_H * 0.78,
                hex(fg, if reversed { BACKGROUND } else { FOREGROUND }),
                escape(symbol)
            );
        }
    }
    out.push_str("</svg>\n");
    out
}

fn main() {
    let mut app = App::new(sample_snapshot(), AppOptions::default());
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("test terminal");
    terminal
        .draw(|frame| kglass::ui::draw(frame, &mut app))
        .expect("draw");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/screenshot.svg");
    std::fs::write(path, svg(terminal.backend().buffer())).expect("write svg");
    println!("wrote {path}");
}
