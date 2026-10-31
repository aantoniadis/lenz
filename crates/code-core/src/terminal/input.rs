#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::keymap::KeyPress;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;

pub(crate) const MAX_PASTE: usize = 1024 * 1024;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn encode_paste(text: &str, bracketed: bool) -> Result<Vec<u8>, String> {
    if text.len() > MAX_PASTE {
        return Err("Paste exceeds the 1 MiB terminal limit; nothing was sent".into());
    }
    for c in text.chars() {
        if matches!(c, '\r' | '\n' | '\t') {
            if !bracketed {
                return Err("This program has not enabled bracketed paste; multiline text and tabs were not sent".into());
            }
        } else if c.is_control() {
            return Err("Terminal paste contains control characters; nothing was sent".into());
        }
    }
    let mut bytes = Vec::with_capacity(text.len() + if bracketed { 12 } else { 0 });
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    bytes.extend_from_slice(text.as_bytes());
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    Ok(bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn quote_paths(paths: &[PathBuf]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Choose a file path to paste".into());
    }
    let mut quoted = String::new();
    for path in paths {
        let text = path
            .to_str()
            .ok_or("This file path cannot be represented as terminal text")?;
        if !path.is_absolute() || text.chars().any(char::is_control) {
            return Err(
                "Choose an absolute file path without control characters; nothing was sent".into(),
            );
        }
        // Count the shell quoting expansion before allocating the next path.
        let size = text
            .len()
            .saturating_add(
                text.bytes()
                    .filter(|c| *c == b'\'')
                    .count()
                    .saturating_mul(3),
            )
            .saturating_add(3);
        if quoted.len().saturating_add(size) > MAX_PASTE {
            return Err("File paths exceed the 1 MiB terminal limit; nothing was sent".into());
        }
        if !quoted.is_empty() {
            quoted.push(' ');
        }
        quoted.push('\'');
        quoted.push_str(&text.replace('\'', "'\\''"));
        quoted.push('\'');
    }
    Ok(quoted)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn encode_key(
    key: &KeyPress,
    application_cursor: bool,
    application_keypad: bool,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(key.shift) + 2 * u8::from(key.alt) + 4 * u8::from(key.ctrl);
    if let Some(name) = key.key.strip_prefix("kp") {
        let (plain, final_byte) = match name {
            "0" => ("0", 'p'),
            "1" => ("1", 'q'),
            "2" => ("2", 'r'),
            "3" => ("3", 's'),
            "4" => ("4", 't'),
            "5" => ("5", 'u'),
            "6" => ("6", 'v'),
            "7" => ("7", 'w'),
            "8" => ("8", 'x'),
            "9" => ("9", 'y'),
            "enter" => ("enter", 'M'),
            "add" => ("+", 'k'),
            "subtract" => ("-", 'm'),
            "multiply" => ("*", 'j'),
            "divide" => ("/", 'o'),
            "decimal" => (".", 'n'),
            "equal" => ("=", 'X'),
            "separator" => (",", 'l'),
            _ => return None,
        };
        if application_keypad {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{final_byte}")
                } else {
                    format!("\x1bO{final_byte}")
                }
                .into_bytes(),
            );
        }
        return encode_key(
            &KeyPress {
                key: plain.into(),
                ..key.clone()
            },
            application_cursor,
            false,
        );
    }
    let final_byte = match key.key.as_str() {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(c) = final_byte {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{c}")
            } else if application_cursor {
                format!("\x1bO{c}")
            } else {
                format!("\x1b[{c}")
            }
            .into_bytes(),
        );
    }
    let function = match key.key.as_str() {
        "f1" => Some('P'),
        "f2" => Some('Q'),
        "f3" => Some('R'),
        "f4" => Some('S'),
        _ => None,
    };
    if let Some(c) = function {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{c}")
            } else {
                format!("\x1bO{c}")
            }
            .into_bytes(),
        );
    }
    let tilde = match key.key.as_str() {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(
            if modifier > 1 {
                format!("\x1b[{n};{modifier}~")
            } else {
                format!("\x1b[{n}~")
            }
            .into_bytes(),
        );
    }
    let mut bytes = match key.key.as_str() {
        "enter" => vec![b'\r'],
        "escape" | "esc" => vec![27],
        "backspace" => vec![127],
        "tab" if key.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        // Host emulators report Ctrl+Space as the space character itself.
        "space" | " " if key.ctrl => vec![0],
        "space" | " " => vec![b' '],
        _ => {
            let mut chars = key.key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if key.ctrl {
                let b = match c {
                    'a'..='z' => c as u8 - b'a' + 1,
                    '@' | '2' | '`' => 0,
                    '[' | '3' => 27,
                    '\\' | '4' => 28,
                    ']' | '5' => 29,
                    '^' | '6' => 30,
                    '_' | '7' | '/' => 31,
                    '?' | '8' => 127,
                    _ => return None,
                };
                vec![b]
            } else {
                let c = if key.shift {
                    match c {
                        'a'..='z' => c.to_ascii_uppercase(),
                        '1' => '!',
                        '2' => '@',
                        '3' => '#',
                        '4' => '$',
                        '5' => '%',
                        '6' => '^',
                        '7' => '&',
                        '8' => '*',
                        '9' => '(',
                        '0' => ')',
                        '-' => '_',
                        '=' => '+',
                        '[' => '{',
                        ']' => '}',
                        ';' => ':',
                        '\'' => '"',
                        ',' => '<',
                        '.' => '>',
                        '/' => '?',
                        '\\' => '|',
                        '`' => '~',
                        _ => c,
                    }
                } else {
                    c
                };
                c.to_string().into_bytes()
            }
        }
    };
    if key.alt {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn terminal_ctrl_space_as_reported_by_host_emulators_is_nul() {
        for key in ["space", " "] {
            let press = KeyPress::new(key, true, false, false).unwrap();
            assert_eq!(encode_key(&press, false, false), Some(vec![0]));
            let press = KeyPress::new(key, false, false, false).unwrap();
            assert_eq!(encode_key(&press, false, false), Some(vec![b' ']));
        }
    }

    #[test]
    fn terminal_paste_preserves_text_and_never_adds_enter() {
        assert_eq!(encode_paste("select 1;", false).unwrap(), b"select 1;");
        assert_eq!(
            encode_paste("select 1;\nselect 2;", true).unwrap(),
            b"\x1b[200~select 1;\nselect 2;\x1b[201~"
        );
        for text in ["one\ntwo", "one\rtwo", "one\ttwo"] {
            assert!(encode_paste(text, false).is_err());
        }
    }

    #[test]
    fn terminal_paste_rejects_injected_controls_and_oversize_atomically() {
        for text in [
            "\x1b[201~echo surprise\n",
            "\0",
            "\x03",
            "\x7f",
            "\u{009b}m",
        ] {
            assert!(
                encode_paste(text, true).is_err(),
                "control payload accepted"
            );
        }
        assert!(encode_paste(&"x".repeat(1024 * 1024 + 1), true).is_err());
    }

    #[test]
    fn terminal_paths_are_literal_and_reject_control_characters() {
        assert_eq!(
            quote_paths(&[
                PathBuf::from("/tmp/it's here.sql"),
                PathBuf::from("/tmp/-script.sh")
            ])
            .unwrap(),
            "'/tmp/it'\\''s here.sql' '/tmp/-script.sh'"
        );
        assert!(quote_paths(&[PathBuf::from("/tmp/first"), PathBuf::from("/tmp/a\nb")]).is_err());
        assert!(quote_paths(&[]).is_err());
    }

    #[test]
    fn terminal_keys_use_application_cursor_and_keep_control_keys_in_child() {
        let key = |name: &str, ctrl, shift, alt| KeyPress::new(name, ctrl, shift, alt).unwrap();
        assert_eq!(
            encode_key(&key("up", false, false, false), false, false).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key(&key("up", false, false, false), true, false).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(&key("c", true, false, false), false, false).unwrap(),
            b"\x03"
        );
        assert_eq!(
            encode_key(&key("z", true, false, false), false, false).unwrap(),
            b"\x1a"
        );
        assert_eq!(
            encode_key(&key("left", true, false, false), true, false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(&key("tab", false, true, false), false, false).unwrap(),
            b"\x1b[Z"
        );
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod keypad_tests {
    use super::*;
    #[test]
    fn terminal_keypad_follows_negotiated_mode() {
        let key = |name| KeyPress::new(name, false, false, false).unwrap();
        assert_eq!(encode_key(&key("kp1"), false, true).unwrap(), b"\x1bOq");
        assert_eq!(encode_key(&key("kp1"), true, false).unwrap(), b"1");
        assert_eq!(encode_key(&key("kpenter"), false, true).unwrap(), b"\x1bOM");
        assert_eq!(encode_key(&key("kpenter"), false, false).unwrap(), b"\r");
        assert_eq!(encode_key(&key("kpadd"), false, false).unwrap(), b"+");
    }
}
