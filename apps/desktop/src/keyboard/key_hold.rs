//! A physical hold is independent of the previous recording's settlement.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KeyHold {
    #[default]
    Released,
    Waiting,
    Consumed,
}

impl KeyHold {
    pub fn press(&mut self) -> bool {
        if *self != Self::Released {
            return false;
        }
        *self = Self::Waiting;
        true
    }
    pub fn release(&mut self) {
        *self = Self::Released;
    }
    pub fn consume(&mut self) {
        if self.pressed() {
            *self = Self::Consumed;
        }
    }
    pub fn pressed(self) -> bool {
        self != Self::Released
    }
    pub fn waiting(self) -> bool {
        self == Self::Waiting
    }
    pub fn consumed(self) -> bool {
        self == Self::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::KeyHold;

    #[test]
    fn repeated_edges_do_not_create_another_recording() {
        let mut key = KeyHold::default();
        assert!(key.press());
        assert!(!key.press());
        key.consume();
        assert!(!key.press());
        assert!(!key.waiting());
        key.release();
        key.release();
        assert!(key.press());
    }

    #[test]
    fn a_new_hold_waits_for_the_previous_recording_and_release_revokes_it() {
        let mut key = KeyHold::default();
        key.press();
        key.consume();
        key.release();
        key.press();
        assert!(key.waiting());
        key.release();
        assert!(!key.waiting());
        assert!(!key.consumed());
    }

    #[test]
    fn an_explicit_action_requires_release_before_another_recording() {
        let mut key = KeyHold::Waiting;
        key.consume();
        assert!(key.pressed());
        assert!(!key.waiting());
        assert!(!key.press());
        key.release();
        assert!(key.press());
    }
}
