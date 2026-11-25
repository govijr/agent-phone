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

/// A Mac key event for iPhone Mirroring: a virtual key code, down or up, and the modifier flags
/// held at that moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacKey {
    pub code: u16,
    pub down: bool,
    pub flags: u64,
}

fn mac_code(c: char) -> Option<(u16, bool)> {
    let (base, shift) = match c {
        'A'..='Z' => (c.to_ascii_lowercase(), true),
        '!' => ('1', true),
        '@' => ('2', true),
        '#' => ('3', true),
        '$' => ('4', true),
        '%' => ('5', true),
        '^' => ('6', true),
        '&' => ('7', true),
        '*' => ('8', true),
        '(' => ('9', true),
        ')' => ('0', true),
        '_' => ('-', true),
        '+' => ('=', true),
        ':' => (';', true),
        '"' => ('\'', true),
        '<' => (',', true),
        '>' => ('.', true),
        '?' => ('/', true),
        '~' => ('`', true),
        '{' => ('[', true),
        '}' => (']', true),
        '|' => ('\\', true),
        other => (other, false),
    };
    let code = match base {
        'a' => 0, 's' => 1, 'd' => 2, 'f' => 3, 'h' => 4, 'g' => 5, 'z' => 6, 'x' => 7, 'c' => 8,
        'v' => 9, 'b' => 11, 'q' => 12, 'w' => 13, 'e' => 14, 'r' => 15, 'y' => 16, 't' => 17,
        '1' => 18, '2' => 19, '3' => 20, '4' => 21, '6' => 22, '5' => 23, '=' => 24, '9' => 25,
        '7' => 26, '-' => 27, '8' => 28, '0' => 29, ']' => 30, 'o' => 31, 'u' => 32, '[' => 33,
        'i' => 34, 'p' => 35, '\n' => 36, 'l' => 37, 'j' => 38, '\'' => 39, 'k' => 40, ';' => 41,
        '\\' => 42, ',' => 43, '/' => 44, 'n' => 45, 'm' => 46, '.' => 47, '\t' => 48, ' ' => 49,
        '`' => 50,
        _ => return None,
    };
    Some((code, shift))
}

fn modifier(name: &str) -> Option<(u16, u64)> {
    Some(match name {
        "cmd" => (55, 0x10_0000),
        "shift" => (56, 0x2_0000),
        "alt" | "option" => (58, 0x8_0000),
        "ctrl" => (59, 0x4_0000),
        _ => return None,
    })
}

/// A key pressed while holding modifiers. iPhone Mirroring passes raw key codes to iOS and drops
/// event flags, so each modifier is held down as its own key. The flag is cleared before the
/// modifier's key-up, or the Mac reads it as still held.
fn mac_press(mods: &[&str], code: u16) -> Vec<MacKey> {
    let mut events = Vec::new();
    let mut flags = 0;
    let held: Vec<(u16, u64)> = mods.iter().filter_map(|m| modifier(m)).collect();
    for (key, flag) in &held {
        flags |= flag;
        events.push(MacKey { code: *key, down: true, flags });
    }
    events.push(MacKey { code, down: true, flags });
    events.push(MacKey { code, down: false, flags });
    for (key, flag) in held.iter().rev() {
        flags &= !flag;
        events.push(MacKey { code: *key, down: false, flags });
    }
    events
}

/// Key events that type `text` on a US layout, or None if a character has no key.
pub fn mac_text(text: &str) -> Option<Vec<MacKey>> {
    let mut events = Vec::new();
    for c in text.chars() {
        let (code, shift) = mac_code(c)?;
        events.extend(mac_press(if shift { &["shift"] } else { &[] }, code));
    }
    Some(events)
}

/// Modifiers plus a character key, such as cmd+v.
pub fn mac_chord(mods: &[&str], c: char) -> Option<Vec<MacKey>> {
    let (code, _) = mac_code(c)?;
    Some(mac_press(mods, code))
}

/// A named key on the Mac keyboard.
pub fn mac_named(name: &str) -> Option<Vec<MacKey>> {
    let code = match name.to_lowercase().as_str() {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" | "delete" => 51,
        "escape" | "esc" => 53,
        "left" | "arrowleft" => 123,
        "right" | "arrowright" => 124,
        "down" | "arrowdown" => 125,
        "up" | "arrowup" => 126,
        _ => return None,
    };
    Some(mac_press(&[], code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_shift_is_held_and_released_cleanly() {
        let events = mac_text("A").unwrap();
        let summary: Vec<(u16, bool, u64)> = events.iter().map(|e| (e.code, e.down, e.flags)).collect();
        assert_eq!(summary, vec![(56, true, 0x2_0000), (0, true, 0x2_0000), (0, false, 0x2_0000), (56, false, 0)]);
    }

    #[test]
    fn mac_paste_chord_holds_command() {
        let events = mac_chord(&["cmd"], 'v').unwrap();
        assert_eq!(events.first().unwrap().code, 55);
        assert_eq!(events[1], MacKey { code: 9, down: true, flags: 0x10_0000 });
        assert_eq!(events.last().unwrap().flags, 0);
    }

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
