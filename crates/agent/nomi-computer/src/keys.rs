//! Parse xdotool-style key combos ("cmd+shift+t") into enigo keys.
//!
//! Aliases are case-insensitive. Single characters map to `Key::Unicode`;
//! ASCII letters are lowercased because shift is expressed as an explicit
//! modifier, not via capitalization.

use enigo::Key;

/// Parse a "+"-separated key combo into the keys to press, in input order.
/// The caller presses them front-to-back and releases back-to-front.
pub fn parse_key_combo(combo: &str) -> Result<Vec<Key>, String> {
    let trimmed = combo.trim();
    if trimmed.is_empty() {
        return Err("Key combo is empty. Provide e.g. \"enter\" or \"cmd+shift+t\".".to_string());
    }

    let mut keys = Vec::new();
    for token in trimmed.split('+') {
        let token = token.trim();
        if token.is_empty() {
            return Err(format!(
                "Malformed key combo {combo:?}: empty segment between '+' separators."
            ));
        }
        keys.push(parse_single_key(token)?);
    }
    Ok(keys)
}

/// Parse one token (a named key alias or a single character).
fn parse_single_key(token: &str) -> Result<Key, String> {
    let lower = token.to_ascii_lowercase();
    let key = match lower.as_str() {
        // Modifiers.
        //
        // `cmd`/`command` is the macOS-idiomatic "primary" accelerator. Models
        // trained on macOS habitually emit "cmd+c"/"cmd+a"/"cmd+shift+t" for
        // copy/select-all/reopen-tab. On macOS that is the Command key (Meta);
        // on Windows/Linux the equivalent accelerator is Control. Mapping `cmd`
        // to Meta everywhere is wrong off macOS — enigo's `Key::Meta` is the
        // Win/Super key there, so "cmd+c" would fire Win+C (Copilot) instead of
        // copy. Remap `cmd`/`command` per-platform.
        //
        // `super`/`win`/`meta` stay Meta on every platform: a model that asks
        // for those explicitly means the OS/Super key, not the accelerator.
        #[cfg(target_os = "macos")]
        "cmd" | "command" => Key::Meta,
        #[cfg(not(target_os = "macos"))]
        "cmd" | "command" => Key::Control,
        "super" | "win" | "meta" => Key::Meta,
        "ctrl" | "control" => Key::Control,
        "alt" | "option" | "opt" => Key::Alt,
        "shift" => Key::Shift,
        // Whitespace / editing
        "enter" | "return" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        // Navigation
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" | "pgup" => Key::PageUp,
        "pagedown" | "page_down" | "pgdn" => Key::PageDown,
        // Function keys
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => {
            let mut chars = token.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Unicode(c.to_ascii_lowercase()),
                _ => {
                    return Err(format!(
                        "Unknown key {token:?}. Use a single character or one of: \
                         cmd, ctrl, alt, shift, enter, esc, tab, space, backspace, \
                         delete, up, down, left, right, home, end, pageup, pagedown, f1-f12."
                    ));
                }
            }
        }
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_named_key() {
        assert_eq!(parse_key_combo("enter").unwrap(), vec![Key::Return]);
        assert_eq!(parse_key_combo("return").unwrap(), vec![Key::Return]);
        assert_eq!(parse_key_combo("esc").unwrap(), vec![Key::Escape]);
        assert_eq!(parse_key_combo("escape").unwrap(), vec![Key::Escape]);
        assert_eq!(parse_key_combo("tab").unwrap(), vec![Key::Tab]);
        assert_eq!(parse_key_combo("space").unwrap(), vec![Key::Space]);
        assert_eq!(parse_key_combo("backspace").unwrap(), vec![Key::Backspace]);
        assert_eq!(parse_key_combo("delete").unwrap(), vec![Key::Delete]);
    }

    #[test]
    fn arrow_and_navigation_keys() {
        assert_eq!(parse_key_combo("up").unwrap(), vec![Key::UpArrow]);
        assert_eq!(parse_key_combo("down").unwrap(), vec![Key::DownArrow]);
        assert_eq!(parse_key_combo("left").unwrap(), vec![Key::LeftArrow]);
        assert_eq!(parse_key_combo("right").unwrap(), vec![Key::RightArrow]);
        assert_eq!(parse_key_combo("home").unwrap(), vec![Key::Home]);
        assert_eq!(parse_key_combo("end").unwrap(), vec![Key::End]);
        assert_eq!(parse_key_combo("pageup").unwrap(), vec![Key::PageUp]);
        assert_eq!(parse_key_combo("pagedown").unwrap(), vec![Key::PageDown]);
    }

    #[test]
    fn modifier_aliases() {
        // `cmd`/`command` is the macOS primary accelerator; on macOS it is Meta,
        // elsewhere it is Control (enigo's Meta is Win/Super off macOS).
        #[cfg(target_os = "macos")]
        let cmd_key = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let cmd_key = Key::Control;
        for alias in ["cmd", "command"] {
            assert_eq!(parse_key_combo(alias).unwrap(), vec![cmd_key], "{alias}");
        }
        // `super`/`win`/`meta` are the OS/Super key on every platform.
        for alias in ["super", "win", "meta"] {
            assert_eq!(parse_key_combo(alias).unwrap(), vec![Key::Meta], "{alias}");
        }
        for alias in ["ctrl", "control"] {
            assert_eq!(
                parse_key_combo(alias).unwrap(),
                vec![Key::Control],
                "{alias}"
            );
        }
        for alias in ["alt", "option", "opt"] {
            assert_eq!(parse_key_combo(alias).unwrap(), vec![Key::Alt], "{alias}");
        }
        assert_eq!(parse_key_combo("shift").unwrap(), vec![Key::Shift]);
    }

    #[test]
    fn function_keys() {
        assert_eq!(parse_key_combo("f1").unwrap(), vec![Key::F1]);
        assert_eq!(parse_key_combo("F5").unwrap(), vec![Key::F5]);
        assert_eq!(parse_key_combo("f12").unwrap(), vec![Key::F12]);
        assert!(parse_key_combo("f13").is_err());
    }

    #[test]
    fn single_character_key() {
        assert_eq!(parse_key_combo("a").unwrap(), vec![Key::Unicode('a')]);
        assert_eq!(parse_key_combo("/").unwrap(), vec![Key::Unicode('/')]);
        assert_eq!(parse_key_combo("0").unwrap(), vec![Key::Unicode('0')]);
    }

    #[test]
    fn uppercase_single_char_is_lowercased() {
        // Shift is an explicit modifier; "T" alone means the 't' key.
        assert_eq!(parse_key_combo("T").unwrap(), vec![Key::Unicode('t')]);
    }

    #[test]
    fn combo_preserves_modifier_order() {
        // `cmd` resolves per-platform (Meta on macOS, Control elsewhere).
        #[cfg(target_os = "macos")]
        let cmd_key = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let cmd_key = Key::Control;
        assert_eq!(
            parse_key_combo("cmd+shift+t").unwrap(),
            vec![cmd_key, Key::Shift, Key::Unicode('t')]
        );
        assert_eq!(
            parse_key_combo("shift+cmd+t").unwrap(),
            vec![Key::Shift, cmd_key, Key::Unicode('t')]
        );
    }

    #[test]
    fn combo_case_insensitive_aliases() {
        #[cfg(target_os = "macos")]
        let cmd_key = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let cmd_key = Key::Control;
        assert_eq!(
            parse_key_combo("CMD+SHIFT+Enter").unwrap(),
            vec![cmd_key, Key::Shift, Key::Return]
        );
        assert_eq!(
            parse_key_combo("Ctrl+Alt+Delete").unwrap(),
            vec![Key::Control, Key::Alt, Key::Delete]
        );
    }

    #[test]
    fn combo_with_whitespace_around_tokens() {
        #[cfg(target_os = "macos")]
        let cmd_key = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let cmd_key = Key::Control;
        assert_eq!(
            parse_key_combo(" cmd + t ").unwrap(),
            vec![cmd_key, Key::Unicode('t')]
        );
    }

    #[test]
    fn empty_string_is_error() {
        assert!(parse_key_combo("").is_err());
        assert!(parse_key_combo("   ").is_err());
    }

    #[test]
    fn empty_segment_is_error() {
        assert!(parse_key_combo("cmd+").is_err());
        assert!(parse_key_combo("+t").is_err());
        assert!(parse_key_combo("cmd++t").is_err());
    }

    #[test]
    fn unknown_key_is_error_and_names_token() {
        let err = parse_key_combo("cmd+bogus").unwrap_err();
        assert!(err.contains("bogus"), "error should name the token: {err}");
    }
}
