//! Keys that ask for a second press: the first press only shows "Press
//! again" on the key for a few seconds, and a second press within that time
//! runs it. For keys such as Restart Home Assistant, so a press by accident
//! does nothing. It works for presses from anywhere: the pad, a pad on
//! screen, a dashboard card or Home Assistant.

use crate::model::{lock, Action, DockKey, Shared};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the key waits for its second press.
pub const WINDOW: Duration = Duration::from_secs(3);
/// What the key says meanwhile.
pub const PROMPT: &str = "Press again";
/// Its background meanwhile.
pub const PROMPT_COLOR: [u8; 3] = [222, 138, 0];

/// The key waiting for its second press: which pad, profile, page and key.
#[derive(Clone, Debug, PartialEq)]
pub struct Armed {
    pub pad_id: String,
    pub profile: usize,
    pub page: usize,
    pub slot: usize,
    pub at: Instant,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);

/// Whether a key asks for a second press.
pub fn needs(key: &DockKey) -> bool {
    key.actions.iter().any(|action| matches!(action, Action::HomeAssistant(binding) if binding.confirm && !binding.service.trim().is_empty()))
}

/// Whether this press runs the key: the second press on the same key within
/// the window. Otherwise it becomes the key waiting, in place of any other.
pub fn decide(armed: &mut Option<Armed>, press: Armed) -> bool {
    let second = armed.as_ref().is_some_and(|waiting| {
        (&waiting.pad_id, waiting.profile, waiting.page, waiting.slot) == (&press.pad_id, press.profile, press.page, press.slot)
            && press.at.saturating_duration_since(waiting.at) < WINDOW
    });
    *armed = if second { None } else { Some(press) };
    second
}

/// A press on a key that asks for a second one. True: run it now. False:
/// it now shows "Press again", and goes back by itself after the window.
pub fn press(shared: &Arc<Shared>, pad_id: &str, profile: usize, page: usize, slot: usize) -> bool {
    let press = Armed { pad_id: pad_id.to_string(), profile, page, slot, at: Instant::now() };
    let run = decide(&mut lock(&ARMED), press);
    redraw(shared);
    if !run {
        let weak = Arc::downgrade(shared);
        std::thread::spawn(move || {
            std::thread::sleep(WINDOW + Duration::from_millis(50));
            if let Some(shared) = weak.upgrade() {
                redraw(&shared);
            }
        });
    }
    run
}

fn redraw(shared: &Shared) {
    crate::pad::wake();
    shared.touch();
}

/// The key of this pad's profile and page that's waiting for its second
/// press, if any.
pub fn waiting(pad_id: &str, profile: usize, page: usize) -> Option<usize> {
    lock(&ARMED)
        .as_ref()
        .filter(|armed| armed.pad_id == pad_id && armed.profile == profile && armed.page == page && armed.at.elapsed() < WINDOW)
        .map(|armed| armed.slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HaAction;

    fn at(slot: usize, after_ms: u64, start: Instant) -> Armed {
        Armed { pad_id: "pad".into(), profile: 0, page: 0, slot, at: start + Duration::from_millis(after_ms) }
    }

    #[test]
    fn the_second_press_within_the_window_runs_it() {
        let start = Instant::now();
        let mut armed = None;
        assert!(!decide(&mut armed, at(2, 0, start)), "the first press only asks");
        assert!(decide(&mut armed, at(2, 1500, start)), "the second runs it");
        assert!(armed.is_none());
        assert!(!decide(&mut armed, at(2, 2000, start)), "and the next one asks again");
    }

    #[test]
    fn too_late_or_another_key_asks_again() {
        let start = Instant::now();
        let mut armed = None;
        decide(&mut armed, at(2, 0, start));
        assert!(!decide(&mut armed, at(2, 3500, start)), "after the window it asks again");
        assert!(!decide(&mut armed, at(3, 3600, start)), "another key asks for itself");
        assert_eq!(armed.as_ref().map(|armed| armed.slot), Some(3));
        let mut other_page = at(3, 3700, start);
        other_page.page = 1;
        assert!(!decide(&mut armed, other_page), "the same key on another page is another key");
    }

    #[test]
    fn only_keys_that_ask_for_it_wait() {
        let mut key = DockKey { actions: vec![Action::HomeAssistant(HaAction::restart_home_assistant())], ..Default::default() };
        assert!(needs(&key));
        key.actions = vec![Action::HomeAssistant(HaAction { entity: "light.desk".into(), service: "toggle".into(), ..Default::default() })];
        assert!(!needs(&key));
        key.actions = vec![Action::HomeAssistant(HaAction { confirm: true, ..Default::default() })];
        assert!(!needs(&key), "nothing to run, nothing to confirm");
    }
}
