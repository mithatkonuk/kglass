//! Colour and no-colour styles. Status is never conveyed by colour alone;
//! the plain theme uses only bold, underline and reverse video.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub ok: Style,
    pub warn: Style,
    pub err: Style,
    pub muted: Style,
    pub accent: Style,
    pub key: Style,
    pub title: Style,
    pub derived: Style,
    pub cursor_focused: Style,
    pub cursor_unfocused: Style,
    pub border_focused: Style,
    pub border: Style,
}

impl Theme {
    pub fn color() -> Self {
        Theme {
            ok: Style::new().fg(Color::Green),
            warn: Style::new().fg(Color::Yellow),
            err: Style::new().fg(Color::Red),
            muted: Style::new().fg(Color::DarkGray),
            accent: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            key: Style::new().fg(Color::Cyan),
            title: Style::new().add_modifier(Modifier::BOLD),
            derived: Style::new().fg(Color::Magenta),
            cursor_focused: Style::new().fg(Color::White).bg(Color::Blue),
            cursor_unfocused: Style::new().bg(Color::DarkGray),
            border_focused: Style::new().fg(Color::Green),
            border: Style::new().fg(Color::DarkGray),
        }
    }

    pub fn plain() -> Self {
        let none = Style::new();
        Theme {
            ok: none,
            warn: Style::new().add_modifier(Modifier::BOLD),
            err: Style::new().add_modifier(Modifier::BOLD),
            muted: none,
            accent: Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            key: Style::new().add_modifier(Modifier::BOLD),
            title: Style::new().add_modifier(Modifier::BOLD),
            derived: none,
            cursor_focused: Style::new().add_modifier(Modifier::REVERSED),
            cursor_unfocused: Style::new().add_modifier(Modifier::UNDERLINED),
            border_focused: Style::new().add_modifier(Modifier::BOLD),
            border: none,
        }
    }

    pub fn for_plain(plain: bool) -> Self {
        if plain {
            Theme::plain()
        } else {
            Theme::color()
        }
    }
}
