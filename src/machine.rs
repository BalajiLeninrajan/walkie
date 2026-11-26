//! What a key press or release means during a recording.
//!
//! Hold to talk: the release ends the recording. A tap shorter than
//! `lock_ms` locks it instead; the next press-and-release ends it. Text is
//! only ever typed on a release, so the user's finger is off the key when
//! keystrokes arrive.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do (key repeat, or the press that will end a lock).
    None,
    /// The tap was short: keep recording until the next press-and-release.
    Lock,
    /// Stop recording and transcribe.
    Finish,
}

#[derive(Debug)]
pub struct KeyMachine {
    started_ms: u64,
    lock_ms: u64,
    locked: bool,
    finish_on_release: bool,
}

impl KeyMachine {
    /// A recording that started (key pressed) at `started_ms`.
    pub fn new(started_ms: u64, lock_ms: u64) -> Self {
        Self { started_ms, lock_ms, locked: false, finish_on_release: false }
    }

    #[cfg(test)]
    pub fn locked(&self) -> bool {
        self.locked
    }

    pub fn press(&mut self) -> Action {
        if self.locked {
            self.finish_on_release = true;
        }
        Action::None
    }

    pub fn release(&mut self, now_ms: u64) -> Action {
        if self.locked {
            return if self.finish_on_release { Action::Finish } else { Action::None };
        }
        if self.lock_ms > 0 && now_ms.saturating_sub(self.started_ms) < self.lock_ms {
            self.locked = true;
            return Action::Lock;
        }
        Action::Finish
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_then_release_finishes() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.press(), Action::None); // key repeat
        assert_eq!(m.release(2500), Action::Finish);
    }

    #[test]
    fn tap_locks_and_next_press_release_finishes() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1100), Action::Lock);
        assert!(m.locked());
        // A stray release without a press does nothing.
        assert_eq!(m.release(5000), Action::None);
        assert_eq!(m.press(), Action::None);
        assert_eq!(m.release(9000), Action::Finish);
    }

    #[test]
    fn release_exactly_at_threshold_finishes() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1300), Action::Finish);
    }

    #[test]
    fn lock_disabled() {
        let mut m = KeyMachine::new(1000, 0);
        assert_eq!(m.release(1001), Action::Finish);
    }

    #[test]
    fn clock_going_backwards_does_not_panic() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(900), Action::Lock);
    }
}
