use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LineEditor {
    value: String,
    cursor: usize,
}

impl LineEditor {
    pub(super) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            cursor: value.len(),
            value,
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.value
    }

    pub(super) fn into_string(self) -> String {
        self.value
    }

    fn insert(&mut self, ch: char) {
        self.value.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    fn move_start(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    fn move_left(&mut self) {
        if let Some((idx, _)) = self.value[..self.cursor].char_indices().next_back() {
            self.cursor = idx;
        }
    }

    fn move_right(&mut self) {
        if let Some(ch) = self.value[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    fn backspace(&mut self) {
        if let Some((start, _)) = self.value[..self.cursor].char_indices().next_back() {
            self.value.drain(start..self.cursor);
            self.cursor = start;
        }
    }

    fn delete(&mut self) {
        if let Some(ch) = self.value[self.cursor..].chars().next() {
            self.value.drain(self.cursor..self.cursor + ch.len_utf8());
        }
    }

    fn clear_before_cursor(&mut self) {
        self.value.drain(..self.cursor);
        self.cursor = 0;
    }

    fn clear_after_cursor(&mut self) {
        self.value.truncate(self.cursor);
    }

    fn delete_previous_word(&mut self) {
        let before = &self.value[..self.cursor];
        let trimmed = before.trim_end_matches(char::is_whitespace);
        let word_start = trimmed
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(idx, ch)| idx + ch.len_utf8())
            .unwrap_or(0);
        self.value.drain(word_start..self.cursor);
        self.cursor = word_start;
    }

    pub(super) fn apply_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home => self.move_start(),
            KeyCode::End => self.move_end(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char('a') if ctrl => self.move_start(),
            KeyCode::Char('b') if ctrl => self.move_left(),
            KeyCode::Char('d') if ctrl => self.delete(),
            KeyCode::Char('e') if ctrl => self.move_end(),
            KeyCode::Char('f') if ctrl => self.move_right(),
            KeyCode::Char('h') if ctrl => self.backspace(),
            KeyCode::Char('k') if ctrl => self.clear_after_cursor(),
            KeyCode::Char('u') if ctrl => self.clear_before_cursor(),
            KeyCode::Char('w') if ctrl => self.delete_previous_word(),
            KeyCode::Char(ch) if !ctrl => self.insert(ch),
            _ => return false,
        }
        true
    }

    pub(super) fn cursor_spans(&self) -> Vec<Span<'static>> {
        let (before, after) = self.value.split_at(self.cursor);
        vec![
            Span::raw(before.to_string()),
            Span::styled("_", Style::default().fg(Color::LightCyan)),
            Span::raw(after.to_string()),
        ]
    }
}
