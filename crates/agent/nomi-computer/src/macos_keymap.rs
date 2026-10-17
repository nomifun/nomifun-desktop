//! Resolve ASCII key identities against the current public macOS keyboard
//! layout before Enigo posts them. Enigo's Unicode lookup keeps the last
//! matching keycode, so a keypad duplicate can replace the regular key (e.g.
//! Shift + keypad '=' remains '='). Modifiers and event delivery stay in Enigo.
//!
//! Called on the existing input main queue. This module only reads TIS/layout
//! data; it never selects an input source, creates events or changes a layout.

use std::ffi::c_void;

use enigo::Key;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *const c_void;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const u8,
        virtual_key: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_state: *mut u32,
        capacity: usize,
        actual_length: *mut usize,
        characters: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
}

struct KeyboardLayout {
    // TIS owns the borrowed CFData; retain its source for every translation.
    source: *const c_void,
    bytes: *const u8,
    keyboard_type: u32,
}

impl Drop for KeyboardLayout {
    fn drop(&mut self) {
        // SAFETY: source is a non-null copy-rule TIS reference owned here.
        unsafe { CFRelease(self.source) };
    }
}

impl KeyboardLayout {
    fn current() -> Option<Self> {
        let sources: [unsafe extern "C" fn() -> *const c_void; 3] = [
            TISCopyCurrentKeyboardInputSource,
            TISCopyCurrentKeyboardLayoutInputSource,
            TISCopyCurrentASCIICapableKeyboardLayoutInputSource,
        ];
        for copy_source in sources {
            // SAFETY: public TIS read APIs run on the existing input main queue.
            // No source is selected or modified. Every copy is either retained
            // by KeyboardLayout or released before trying the next fallback.
            unsafe {
                let source = copy_source();
                if source.is_null() { continue; }
                let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
                if !data.is_null() && CFGetTypeID(data) == CFDataGetTypeID()
                    && CFDataGetLength(data) > 0
                {
                    let bytes = CFDataGetBytePtr(data);
                    if !bytes.is_null() {
                        return Some(Self { source, bytes, keyboard_type: u32::from(LMGetKbdType()) });
                    }
                }
                CFRelease(source);
            }
        }
        None
    }

    fn character(&self, keycode: u16, modifiers: u32) -> Option<char> {
        let mut dead_state = 0;
        let mut actual_length = 0usize;
        let mut characters = [0u16; 4];
        // SAFETY: the live TIS source retains its native layout data. Sizes
        // follow public SDK UniCharCount (unsigned long on arm64/x86_64).
        let status = unsafe { UCKeyTranslate(
            self.bytes, keycode, 3, modifiers, self.keyboard_type,
            1, // kUCKeyTranslateNoDeadKeysMask, not its bit index (zero).
            &mut dead_state, characters.len(), &mut actual_length, characters.as_mut_ptr(),
        ) };
        if status != 0 || actual_length != 1 { return None; }
        char::from_u32(u32::from(characters[0]))
    }

    fn ascii_keycode(&self, character: char) -> Option<u16> {
        first_unmodified_match(character, |keycode| self.character(keycode, 0))
    }
}

fn first_unmodified_match(character: char, mut translate: impl FnMut(u16) -> Option<char>) -> Option<u16> {
    // Events.h hardware key groups, including JIS KeypadComma. Character
    // positions come only from the live layout. Some JIS regular keys have
    // higher codes than the keypad, so numeric first-match alone is unsafe.
    let keypad = |code: u16| matches!(code,
        0x41 | 0x43 | 0x45 | 0x47 | 0x4b | 0x4c | 0x4e | 0x51 | 0x52 | 0x53 |
        0x54 | 0x55 | 0x56 | 0x57 | 0x58 | 0x59 | 0x5b | 0x5c | 0x5f
    );
    // A keypad-only character is a valid native match, but it cannot replace
    // any regular key that actually produces the requested unmodified char.
    (0..128).filter(|code| !keypad(*code))
        .chain((0..128).filter(|code| keypad(*code)))
        .find(|keycode| translate(*keycode) == Some(character))
}

fn adapt_keys(keys: Vec<Key>, mut lookup: impl FnMut(char) -> Option<u16>) -> Result<Vec<Key>, String> {
    keys.into_iter().map(|key| match key {
        Key::Unicode(character) if character.is_ascii_graphic() => lookup(character)
            .map(|keycode| Key::Other(u32::from(keycode)))
            .ok_or_else(|| format!(
                "Unsupported ASCII key {character:?}: no safe unmodified key exists in the current macOS layout; use the separate text action for text input."
            )),
        other => Ok(other),
    }).collect()
}

pub(super) fn resolve_ascii_keys(keys: Vec<Key>) -> Result<Vec<Key>, String> {
    if !keys.iter().any(|key| matches!(key, Key::Unicode(character) if character.is_ascii_graphic())) {
        return Ok(keys);
    }
    let layout = KeyboardLayout::current();
    adapt_keys(keys, |character| layout.as_ref().and_then(|layout| layout.ascii_keycode(character)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_layout_matches_keep_first_key_and_exact_modifier_release_identity() {
        let resolved = adapt_keys(vec![Key::Meta, Key::Shift, Key::Unicode('=')], |character| {
            first_unmodified_match(character, |code| match code {
                7 | 99 => Some('='),
                _ => None,
            })
        }).unwrap();
        assert_eq!(resolved, vec![Key::Meta, Key::Shift, Key::Other(7)]);
        assert_eq!(resolved.into_iter().rev().collect::<Vec<_>>(), vec![Key::Other(7), Key::Shift, Key::Meta]);
        // A JIS/custom regular key can occur after a lower keypad code.
        assert_eq!(first_unmodified_match('_', |code| match code {
            0x51 | 0x5e => Some('_'), _ => None,
        }), Some(0x5e));
        // A real keypad-only mapping remains usable rather than inventing a
        // US-layout position or silently emitting an unrelated key zero.
        assert_eq!(first_unmodified_match('*', |code| (code == 0x43).then_some('*')), Some(0x43));
    }

    #[test]
    fn missing_layout_or_character_does_not_fabricate_key_zero_or_submit_partial_combo() {
        let error = adapt_keys(vec![Key::Shift, Key::Unicode('=')], |_| None).unwrap_err();
        assert!(error.contains("no safe unmodified key"));
        assert_eq!(adapt_keys(vec![Key::Meta, Key::Return, Key::Unicode('中')], |_| {
            panic!("named and non-ASCII keys retain the existing native path")
        }).unwrap(), vec![Key::Meta, Key::Return, Key::Unicode('中')]);
    }

    #[test]
    #[ignore = "explicit read-only current-layout acceptance; run this CLI test alone with --test-threads=1"]
    fn native_readonly_current_layout_prefers_regular_equals_and_digit() {
        // No Enigo instance, AX request, event creation/post, layout selection
        // or user window interaction. TextInputSources.h explicitly permits
        // a non-UI CLI to use TIS off main when no other thread accesses TIS.
        // This isolated ignored test must run alone; product calls remain on
        // the existing App main queue. Other layouts need their own expected
        // Shift semantic rather than assuming this host's '=' -> '+'.
        let layout = KeyboardLayout::current().expect("public current layout data");
        let equals = layout.ascii_keycode('=').expect("current equals key");
        let digit = layout.ascii_keycode('1').expect("current digit one key");
        assert_eq!(layout.character(equals, 0), Some('='));
        assert_eq!(layout.character(equals, 2), Some('+'));
        assert_eq!(layout.character(digit, 0), Some('1'));
        for code in 0..equals { assert_ne!(layout.character(code, 0), Some('=')); }
        for code in 0..digit { assert_ne!(layout.character(code, 0), Some('1')); }
        // Public macOS keypad codes; absence is fine for another layout.
        // If it advertises the duplicate, the regular match must win.
        if layout.character(0x51, 0) == Some('=') { assert!(equals < 0x51); }
        if layout.character(0x53, 0) == Some('1') { assert!(digit < 0x53); }
        println!("NATIVE_READONLY_KEYMAP equals_keycode={equals} equals_shift_is_plus=true digit_one_keycode={digit} digit_one_is_regular=true events_created_or_posted=false");
    }
}
