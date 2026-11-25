//! Text and named keys as HID keyboard usages (page 7) on a US layout.

const SHIFT: u32 = 0xE1;
const COMMAND: u32 = 0xE3;
const DOWN: u8 = 1;
const UP: u8 = 2;

/// The usage for a character, and whether it needs shift. None for characters a US keyboard
/// cannot type directly.
fn usage_for(c: char) -> Option<(u32, bool)> {
    Some(match c {
        'a'..='z' => (0x04 + (c as u32 - 'a' as u32), false),
        'A'..='Z' => (0x04 + (c as u32 - 'A' as u32), true),
        '1'..='9' => (0x1E + (c as u32 - '1' as u32), false),
        '0' => (0x27, false),
        '\n' => (0x28, false),
        '\t' => (0x2B, false),
        ' ' => (0x2C, false),
        '-' => (0x2D, false),
        '_' => (0x2D, true),
        '=' => (0x2E, false),
        '+' => (0x2E, true),
        '[' => (0x2F, false),
        '{' => (0x2F, true),
        ']' => (0x30, false),
        '}' => (0x30, true),
        '\\' => (0x31, false),
        '|' => (0x31, true),
        ';' => (0x33, false),
        ':' => (0x33, true),
        '\'' => (0x34, false),
        '"' => (0x34, true),
        '`' => (0x35, false),
        '~' => (0x35, true),
        ',' => (0x36, false),
        '<' => (0x36, true),
        '.' => (0x37, false),
        '>' => (0x37, true),
        '/' => (0x38, false),
        '?' => (0x38, true),
        '!' => (0x1E, true),
        '@' => (0x1F, true),
        '#' => (0x20, true),
        '$' => (0x21, true),
        '%' => (0x22, true),
        '^' => (0x23, true),
        '&' => (0x24, true),
        '*' => (0x25, true),
        '(' => (0x26, true),
        ')' => (0x27, true),
        _ => return None,
    })
}

/// Key events that type `text`, or None if any character is not on a US keyboard.
pub fn text(text: &str) -> Option<Vec<(u32, u8)>> {
    let mut events = Vec::new();
    for c in text.chars() {
        let (usage, shift) = usage_for(c)?;
        if shift {
            events.push((SHIFT, DOWN));
        }
        events.push((usage, DOWN));
        events.push((usage, UP));
        if shift {
            events.push((SHIFT, UP));
        }
    }
    Some(events)
}

/// Command plus a key, such as select all or paste.
pub fn command(c: char) -> Vec<(u32, u8)> {
    let (usage, _) = usage_for(c).expect("command key on the US layout");
    vec![(COMMAND, DOWN), (usage, DOWN), (usage, UP), (COMMAND, UP)]
}

pub fn backspace() -> Vec<(u32, u8)> {
    vec![(0x2A, DOWN), (0x2A, UP)]
}

/// A named key (`enter`, `tab`, `backspace`, arrows…) as a press-and-release.
pub fn named(name: &str) -> Option<Vec<(u32, u8)>> {
    let usage = match name.to_lowercase().as_str() {
        "enter" | "return" => 0x28,
        "escape" | "esc" => 0x29,
        "backspace" | "delete" => 0x2A,
        "tab" => 0x2B,
        "space" => 0x2C,
        "right" | "arrowright" => 0x4F,
        "left" | "arrowleft" => 0x50,
        "down" | "arrowdown" => 0x51,
        "up" | "arrowup" => 0x52,
        _ => return None,
    };
    Some(vec![(usage, DOWN), (usage, UP)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_shifted_characters() {
        assert_eq!(
            text("A!").unwrap(),
            vec![(SHIFT, 1), (0x04, 1), (0x04, 2), (SHIFT, 2), (SHIFT, 1), (0x1E, 1), (0x1E, 2), (SHIFT, 2)]
        );
    }

    #[test]
    fn rejects_characters_off_the_layout() {
        assert!(text("café").is_none());
    }
}
