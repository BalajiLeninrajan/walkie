//! What a key press or release means during a recording.
//!
//! Hold to talk: the release ends the recording. A tap shorter than
//! `lock_ms` locks it instead; the next press-and-release ends it. Text is
//! only ever typed on a release, so the user's finger is off the key when
//! keystrokes arrive.
//!
//! Times are the monotonic clock readings `walkie start` and `walkie stop`
//! took when kanata ran them, not when the session got around to reading
//! the message. kanata starts the two commands as separate processes, so on
//! a quick tap the release can reach the session before the press; the
//! stamps put them back in order.

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
    pressed_ms: u64,
    lock_ms: u64,
    locked: bool,
    finish_on_release: bool,
    /// A release seen while locked with no press before it.
    early_release_ms: Option<u64>,
}

impl KeyMachine {
    /// A recording whose key went down at `pressed_ms`.
    pub fn new(pressed_ms: u64, lock_ms: u64) -> Self {
        Self { pressed_ms, lock_ms, locked: false, finish_on_release: false, early_release_ms: None }
    }

    #[cfg(test)]
    pub fn locked(&self) -> bool {
        self.locked
    }

    pub fn press(&mut self, at_ms: u64) -> Action {
        if !self.locked {
            return Action::None; // key repeat
        }
        // This press's own release already arrived.
        if self.early_release_ms.take().is_some_and(|r| r >= at_ms) {
            return Action::Finish;
        }
        self.finish_on_release = true;
        Action::None
    }

    pub fn release(&mut self, at_ms: u64) -> Action {
        if self.locked {
            if self.finish_on_release {
                return Action::Finish;
            }
            self.early_release_ms = Some(at_ms);
            return Action::None;
        }
        if self.lock_ms > 0 && at_ms.saturating_sub(self.pressed_ms) < self.lock_ms {
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
        assert_eq!(m.press(1500), Action::None); // key repeat
        assert_eq!(m.release(2500), Action::Finish);
    }

    #[test]
    fn tap_locks_and_next_press_release_finishes() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1100), Action::Lock);
        assert!(m.locked());
        assert_eq!(m.press(9000), Action::None);
        assert_eq!(m.release(9100), Action::Finish);
    }

    #[test]
    fn lock_uses_the_senders_clock_not_arrival_time() {
        // The session may read the release late; only the stamps count.
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1250), Action::Lock);
    }

    #[test]
    fn ending_tap_whose_release_arrives_first() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1100), Action::Lock);
        assert_eq!(m.release(9100), Action::None); // arrived before its press
        assert_eq!(m.press(9000), Action::Finish);
    }

    #[test]
    fn stray_release_then_a_normal_tap() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(1100), Action::Lock);
        assert_eq!(m.release(5000), Action::None); // stray, older than the next press
        assert_eq!(m.press(9000), Action::None);
        assert_eq!(m.release(9100), Action::Finish);
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
    fn release_stamped_before_press_locks() {
        let mut m = KeyMachine::new(1000, 300);
        assert_eq!(m.release(900), Action::Lock);
    }
}
