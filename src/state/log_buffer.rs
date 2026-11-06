//! Bounded log line buffer with follow mode.

use std::collections::VecDeque;

use crate::text::SafeText;

/// Holds at most `cap` lines. When following, the view sticks to the newest
/// lines; otherwise it stays on the same content while old lines are evicted.
#[derive(Clone, Debug)]
pub struct LogBuffer {
    lines: VecDeque<SafeText>,
    cap: usize,
    follow: bool,
    /// First visible line when not following (index into `lines`).
    top: usize,
}

impl LogBuffer {
    /// A buffer holding at most `cap` lines (at least one).
    pub fn new(cap: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            cap: cap.max(1),
            follow: true,
            top: 0,
        }
    }

    pub fn push(&mut self, line: SafeText) {
        self.lines.push_back(line);
        if self.lines.len() > self.cap {
            self.lines.pop_front();
            self.top = self.top.saturating_sub(1);
        }
    }

    pub fn extend(&mut self, lines: impl IntoIterator<Item = SafeText>) {
        for line in lines {
            self.push(line);
        }
    }

    /// Drop all lines and return to follow mode (used when the stream is replaced).
    pub fn clear(&mut self) {
        self.lines.clear();
        self.top = 0;
        self.follow = true;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn follow(&self) -> bool {
        self.follow
    }

    /// Turn follow mode on or off. Pausing keeps the lines currently shown in a
    /// viewport of `viewport` rows on screen.
    pub fn set_follow(&mut self, on: bool, viewport: usize) {
        if self.follow && !on {
            self.top = self.top(viewport);
        }
        self.follow = on;
    }

    /// Scroll by `delta` lines in a viewport of `viewport` rows.
    /// Scrolling up leaves follow mode.
    pub fn scroll_by(&mut self, delta: isize, viewport: usize) {
        let max_top = self.lines.len().saturating_sub(viewport);
        self.top = self.top(viewport).saturating_add_signed(delta).min(max_top);
        if delta < 0 {
            self.follow = false;
        }
    }

    /// Index of the first visible line for a viewport of `viewport` rows.
    pub fn top(&self, viewport: usize) -> usize {
        let max_top = self.lines.len().saturating_sub(viewport);
        if self.follow {
            max_top
        } else {
            self.top.min(max_top)
        }
    }

    pub fn visible(&self, viewport: usize) -> impl Iterator<Item = &str> {
        let top = self.top(viewport);
        let end = (top + viewport).min(self.lines.len());
        self.lines.range(top..end).map(SafeText::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(cap: usize, n: usize) -> LogBuffer {
        let mut b = LogBuffer::new(cap);
        b.extend((0..n).map(|i| SafeText::from(i.to_string())));
        b
    }

    fn view(b: &LogBuffer, rows: usize) -> Vec<&str> {
        b.visible(rows).collect()
    }

    #[test]
    fn buffer_caps_and_follows() {
        let b = filled(3, 5);
        assert_eq!(b.len(), 3);
        assert_eq!(view(&b, 2), ["3", "4"]);
    }

    #[test]
    fn not_following_keeps_view_stable_while_evicting() {
        let mut b = filled(5, 5);
        b.scroll_by(-2, 2);
        assert!(!b.follow());
        assert_eq!(view(&b, 2), ["1", "2"]);
        b.push("5".into());
        assert_eq!(view(&b, 2), ["1", "2"]);
        b.extend((6..20).map(|i| SafeText::from(i.to_string())));
        assert_eq!(view(&b, 2), ["15", "16"]);
    }

    #[test]
    fn pausing_follow_keeps_current_view() {
        let mut b = filled(10, 10);
        b.set_follow(false, 3);
        assert_eq!(view(&b, 3), ["7", "8", "9"]);
        b.push(SafeText::from("10"));
        assert_eq!(view(&b, 3), ["7", "8", "9"], "paused view does not move");
    }

    #[test]
    fn refollow_jumps_to_bottom() {
        let mut b = filled(10, 10);
        b.scroll_by(-5, 3);
        b.set_follow(true, 3);
        assert_eq!(view(&b, 3), ["7", "8", "9"]);
    }

    #[test]
    fn scrolling_down_past_end_stops_at_bottom() {
        let mut b = filled(10, 10);
        b.scroll_by(-5, 3);
        b.scroll_by(100, 3);
        assert_eq!(view(&b, 3), ["7", "8", "9"]);
    }

    #[test]
    fn fewer_lines_than_viewport() {
        let b = filled(10, 2);
        assert_eq!(view(&b, 5), ["0", "1"]);
        assert_eq!(b.top(5), 0);
    }

    #[test]
    fn stored_lines_are_sanitized() {
        let mut b = LogBuffer::new(4);
        b.push(SafeText::from("ok \x1b[2Jcleared"));
        assert_eq!(view(&b, 1), ["ok cleared"]);
    }

    #[test]
    fn clear_resets_follow_and_lines() {
        let mut b = filled(10, 10);
        b.scroll_by(-3, 3);
        b.clear();
        assert_eq!(b.len(), 0);
        assert!(b.follow());
        assert_eq!(view(&b, 3), Vec::<&str>::new());
    }
}
