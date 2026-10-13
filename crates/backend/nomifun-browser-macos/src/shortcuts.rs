//! App-local accelerators for the native child, before AppKit window menus.
use objc2_app_kit::{NSEvent, NSEventModifierFlags as Flags, NSEventType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserShortcutAction { Address, NewTab, CloseTab, Reload, Back, Forward }

pub(crate) fn event_action(event: &NSEvent) -> Option<BrowserShortcutAction> {
    // FlagsChanged has no key characters and querying them raises an exception.
    if event.r#type() != NSEventType::KeyDown { return None; }
    let flags = event.modifierFlags();
    action(&event.charactersIgnoringModifiers()?.to_string(), flags.contains(Flags::Command),
        flags.contains(Flags::Control), flags.contains(Flags::Option), flags.contains(Flags::Shift))
}
fn action(key: &str, command: bool, control: bool, option: bool, shift: bool) -> Option<BrowserShortcutAction> {
    if control || shift || command && option { return None; }
    match (key.to_ascii_lowercase().as_str(), command, option) {
        ("l", true, false) => Some(BrowserShortcutAction::Address),
        ("t", true, false) => Some(BrowserShortcutAction::NewTab),
        ("w", true, false) => Some(BrowserShortcutAction::CloseTab),
        ("r", true, false) | ("\u{f708}", false, false) => Some(BrowserShortcutAction::Reload),
        ("\u{f702}", false, true) => Some(BrowserShortcutAction::Back),
        ("\u{f703}", false, true) => Some(BrowserShortcutAction::Forward),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_browser_keys_preserve_edit_and_modified_shortcuts() {
        for (key, expected) in [("L", BrowserShortcutAction::Address),("t",BrowserShortcutAction::NewTab),("w",BrowserShortcutAction::CloseTab),("r",BrowserShortcutAction::Reload)] {
            assert_eq!(action(key, true, false, false, false), Some(expected));
            assert_eq!(action(key, true, true, false, false), None);
            assert_eq!(action(key, true, false, true, false), None);
            assert_eq!(action(key, true, false, false, true), None);
        }
        assert_eq!(action("\u{f708}", false, false, false, false), Some(BrowserShortcutAction::Reload));
        assert_eq!(action("\u{f702}", false, false, true, false), Some(BrowserShortcutAction::Back));
        assert_eq!(action("\u{f703}", false, false, true, false), Some(BrowserShortcutAction::Forward));
        for key in ["a","c","v","x","z","q"] { assert_eq!(action(key, true, false, false, false), None); }
    }
}
