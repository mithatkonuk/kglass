//! Cursor and scroll offset for a table-like list.

/// Keyboard cursor over `len` rows plus the first visible row.
/// All operations are no-ops on an empty list and never index out of range.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListCursor {
    len: usize,
    cursor: usize,
    offset: usize,
}

impl ListCursor {
    /// Update the row count, clamping cursor and offset into range.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor = self.cursor.min(len.saturating_sub(1));
        self.offset = self.offset.min(self.cursor);
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Index of the cursor row, or `None` for an empty list.
    pub fn selected(&self) -> Option<usize> {
        (self.len > 0).then_some(self.cursor)
    }

    /// Move the cursor by `delta` rows, stopping at either end.
    pub fn move_by(&mut self, delta: isize) {
        if self.len == 0 {
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(delta).min(self.len - 1);
    }

    /// Put the cursor on `idx`, clamped to the last row.
    pub fn select(&mut self, idx: usize) {
        if self.len > 0 {
            self.cursor = idx.min(self.len - 1);
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Adjust the offset so the cursor row is inside a viewport of `rows` rows.
    pub fn scroll_into_view(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + rows {
            self.offset = self.cursor + 1 - rows;
        }
        self.offset = self.offset.min(self.len.saturating_sub(rows));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_is_inert() {
        let mut c = ListCursor::default();
        c.move_by(5);
        c.move_by(-5);
        c.select(3);
        assert_eq!(c.selected(), None);
        c.scroll_into_view(10);
        assert_eq!(c.offset(), 0);
    }

    #[test]
    fn cursor_clamps_and_scrolls() {
        let mut c = ListCursor::default();
        c.set_len(10);
        c.move_by(100);
        assert_eq!(c.selected(), Some(9));
        c.scroll_into_view(4);
        assert_eq!(c.offset(), 6);
        c.set_len(3);
        assert_eq!(c.selected(), Some(2));
        c.scroll_into_view(4);
        assert_eq!(c.offset(), 0);
    }

    #[test]
    fn moving_up_scrolls_back() {
        let mut c = ListCursor::default();
        c.set_len(10);
        c.select(9);
        c.scroll_into_view(3);
        c.move_by(-8);
        c.scroll_into_view(3);
        assert_eq!((c.selected(), c.offset()), (Some(1), 1));
    }

    #[test]
    fn zero_viewport_does_not_panic() {
        let mut c = ListCursor::default();
        c.set_len(5);
        c.select(4);
        c.scroll_into_view(0);
        assert_eq!(c.selected(), Some(4));
    }
}
