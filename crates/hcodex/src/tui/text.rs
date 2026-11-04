//! Pure display-width wrapping shared by the composer and scrollable inspectors.

pub(super) fn inspector_width(terminal_width: u16) -> u16 {
    terminal_width.saturating_mul(4) / 5
}

pub(super) fn wrap_display_line(value: &str, width: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for character in value.chars() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if current_width > 0 && current_width.saturating_add(character_width) > width {
            chunks.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(character);
        current_width = current_width.saturating_add(character_width);
    }
    chunks.push(current);
    chunks
}
