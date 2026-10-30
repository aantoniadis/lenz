//! Bounded session activity, retained when transient status messages are cleared.
use std::collections::VecDeque;

#[derive(Debug, Default)]
pub(super) struct Messages {
    pub(super) current: Vec<String>,
    history: VecDeque<String>,
    rows: std::cell::RefCell<Option<(usize, std::sync::Arc<Vec<String>>)>>,
    pub(super) shown: bool,
}
impl std::ops::Deref for Messages {
    type Target = [String];
    fn deref(&self) -> &Self::Target { &self.current }
}
impl Messages {
    pub(super) fn history_text(&self) -> String { self.history.iter().cloned().collect::<Vec<_>>().join("\n") }
    pub(super) fn push(&mut self, text: String) {
        self.record(&text);
        self.current.push(text);
    }
    pub(super) fn record(&mut self, text: &str) {
        let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() % 86400;
        let mut end = text.len().min(8192);
        while !text.is_char_boundary(end) { end -= 1; }
        let clean: String = text.get(..end).unwrap_or_default().chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
        let row = format!("[{:02}:{:02}:{:02} UTC] {}{}", seconds / 3600, seconds / 60 % 60, seconds % 60, clean, if end < text.len() { " [truncated]" } else { "" });
        if self.history.len() == 256 { self.history.pop_front(); }
        self.history.push_back(row);
        *self.rows.get_mut() = None;
    }
    pub(super) fn clear(&mut self) { self.current.clear(); }
    pub(super) fn retain(&mut self, f: impl FnMut(&String) -> bool) { self.current.retain(f); }
    pub(super) fn extend(&mut self, rows: impl IntoIterator<Item = String>) { for row in rows { self.push(row); } }
    pub(super) fn append(&mut self, rows: &mut Vec<String>) { self.extend(rows.drain(..)); }
}
impl From<Vec<String>> for Messages {
    fn from(rows: Vec<String>) -> Self { let mut messages = Self::default(); messages.extend(rows); messages }
}
impl super::State {
    pub fn activity_log(&self) -> &VecDeque<String> { &self.messages.history }
    pub fn activity_log_rows(&self, columns: usize) -> std::sync::Arc<Vec<String>> {
        let columns = columns.max(1);
        let mut cache = self.messages.rows.borrow_mut();
        if let Some((width, rows)) = &*cache && *width == columns { return rows.clone(); }
        let mut rows = Vec::new();
        for entry in &self.messages.history {
            for line in entry.lines() {
                let starts = std::iter::once(0).chain(lenz_content::wrap::wrap_starts(line, columns)).chain(std::iter::once(line.len())).collect::<Vec<_>>();
                for pair in starts.windows(2) { if let Some(row) = line.get(pair[0]..pair[1]) { rows.push(row.to_owned()); } }
            }
        }
        if rows.is_empty() { rows.push("No activity yet.".into()); }
        let rows = std::sync::Arc::new(rows);
        *cache = Some((columns, rows.clone()));
        rows
    }
    pub fn activity_log_shown(&self) -> bool { self.messages.shown }
}
impl super::Session {
    pub(super) fn clear_activity_log(&mut self) { self.state.messages.history.clear(); *self.state.messages.rows.get_mut() = None; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bounded_and_survives_status_clear() {
        let mut m = Messages::default();
        for i in 0..300 { m.push(format!("notice {i}")); }
        m.clear();
        assert!(m.is_empty());
        assert_eq!(m.history.len(), 256);
        assert!(m.history.front().unwrap().ends_with("notice 44"));
        m.record(&"é".repeat(8192));
        assert!(m.history.back().unwrap().ends_with("[truncated]"));
        assert!(!m.shown);
    }
}
