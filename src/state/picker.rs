//! Type-to-filter picker used for switching context and namespace.

use crate::cluster::config::is_valid_namespace;
use crate::cluster::fetch::FetchError;
use crate::text::{SafeText, sanitize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Context,
    Namespace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerItem {
    pub name: SafeText,
    /// Secondary text, e.g. `rancher · rancher.example.com`.
    pub detail: SafeText,
    /// The context or namespace in use now.
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    items: Vec<PickerItem>,
    pub query: String,
    cursor: usize,
    /// Items are still being fetched (namespaces).
    pub loading: bool,
    /// Items could not be fetched; the typed text is accepted instead.
    pub error: Option<FetchError>,
}

impl Picker {
    /// A picker waiting for its items.
    pub fn new(kind: PickerKind) -> Self {
        Picker {
            kind,
            items: Vec::new(),
            query: String::new(),
            cursor: 0,
            loading: true,
            error: None,
        }
    }

    pub fn set_items(&mut self, items: Vec<PickerItem>) {
        self.items = items;
        self.loading = false;
        self.cursor = self.items.iter().position(|i| i.current).unwrap_or(0);
        self.clamp();
    }

    pub fn set_error(&mut self, error: FetchError) {
        self.loading = false;
        self.error = Some(error);
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// Items whose name contains the query, ignoring case.
    pub fn matches(&self) -> Vec<&PickerItem> {
        let query = self.query.to_lowercase();
        self.items
            .iter()
            .filter(|i| i.name.as_str().to_lowercase().contains(&query))
            .collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn move_by(&mut self, delta: isize) {
        self.cursor = self.cursor.saturating_add_signed(delta);
        self.clamp();
    }

    /// Append typed text; control characters are dropped.
    pub fn type_char(&mut self, c: char) {
        self.query.push_str(&sanitize(&c.to_string()));
        self.clamp();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.clamp();
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.matches().len().saturating_sub(1));
    }

    /// The item under the cursor. A namespace picker with no match accepts the
    /// typed name, since listing namespaces is often forbidden.
    pub fn chosen(&self) -> Option<SafeText> {
        if let Some(item) = self.matches().get(self.cursor) {
            return Some(item.name.clone());
        }
        let typed = self.query.trim();
        (self.kind == PickerKind::Namespace && is_valid_namespace(typed))
            .then(|| SafeText::from(typed))
    }

    /// The typed text matches nothing and is not a valid namespace name.
    pub fn invalid_name(&self) -> bool {
        self.kind == PickerKind::Namespace
            && self.matches().is_empty()
            && !self.query.trim().is_empty()
            && !is_valid_namespace(self.query.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> PickerItem {
        PickerItem {
            name: name.into(),
            detail: SafeText::default(),
            current: false,
        }
    }

    fn picker(kind: PickerKind) -> Picker {
        let mut p = Picker::new(kind);
        p.set_items(vec![item("dev-east"), item("dev-west"), item("prod")]);
        p
    }

    fn names(p: &Picker) -> Vec<&str> {
        p.matches().iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn typing_filters_case_insensitively() {
        let mut p = picker(PickerKind::Context);
        for c in "DEV".chars() {
            p.type_char(c);
        }
        assert_eq!(names(&p), ["dev-east", "dev-west"]);
        p.backspace();
        p.backspace();
        p.backspace();
        assert_eq!(names(&p).len(), 3);
    }

    #[test]
    fn cursor_stays_within_matches() {
        let mut p = picker(PickerKind::Context);
        p.move_by(10);
        assert_eq!(p.chosen().unwrap().as_str(), "prod");
        p.type_char('w');
        assert_eq!(
            p.chosen().unwrap().as_str(),
            "dev-west",
            "cursor clamps after filtering"
        );
        p.move_by(-5);
        assert_eq!(p.chosen().unwrap().as_str(), "dev-west");
    }

    #[test]
    fn context_picker_needs_a_match() {
        let mut p = picker(PickerKind::Context);
        p.type_char('z');
        assert_eq!(p.chosen(), None);
    }

    #[test]
    fn namespace_picker_accepts_free_text() {
        let mut p = Picker::new(PickerKind::Namespace);
        assert!(p.loading);
        p.set_error(crate::cluster::fetch::FetchError::Timeout);
        assert!(!p.loading);
        for c in "team-a".chars() {
            p.type_char(c);
        }
        assert_eq!(p.chosen().unwrap().as_str(), "team-a");
    }

    #[test]
    fn free_text_must_be_a_valid_namespace() {
        let mut p = Picker::new(PickerKind::Namespace);
        p.set_items(Vec::new());
        for c in "x/secrets".chars() {
            p.type_char(c);
        }
        assert_eq!(p.chosen(), None);
        assert!(p.invalid_name());
    }

    #[test]
    fn typed_text_is_sanitized() {
        let mut p = Picker::new(PickerKind::Namespace);
        p.set_items(Vec::new());
        p.type_char('\u{1b}');
        p.type_char('a');
        assert_eq!(p.query, "a");
    }
}
