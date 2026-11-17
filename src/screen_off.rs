//! Automatic screen off: a pad's screen goes dark after a while without a
//! press, as the original app offers. The next press
//! only wakes it. It goes dark by brightness 0, which both pads already take,
//! rather than separate off and wake commands, which have never been
//! sent from here. Mini Monitor keeps a pad awake.

use crate::model::{lock, Config, PadKind, Shared};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The original's choices, in milliseconds, with 0 for never. Its last choice,
/// following Windows (`-10087`), isn't offered.
pub const CHOICES: [(u64, &str); 7] = [
    (60_000, "After 1 minute"),
    (300_000, "After 5 minutes"),
    (900_000, "After 15 minutes"),
    (1_800_000, "After 30 minutes"),
    (3_600_000, "After 1 hour"),
    (7_200_000, "After 2 hours"),
    (0, "Never"),
];

/// When each pad was last pressed: the 293S, then the AKP03E.
static LAST_PRESS: Mutex<[Option<Instant>; 2]> = Mutex::new([None, None]);
/// Pads turned off from Home Assistant, by id: dark until turned on again or
/// pressed, whatever the timer says.
static TURNED_OFF: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A virtual pad has no screen of its own to turn off.
fn slot(kind: PadKind) -> Option<usize> {
    match kind {
        PadKind::Main => Some(0),
        PadKind::Akp03e => Some(1),
        PadKind::Dashboard => None,
    }
}

/// The label of the chosen time, or of the nearest choice below it.
pub fn label_of(ms: u64) -> &'static str {
    CHOICES.iter().find(|(choice, _)| *choice == ms).map_or("Never", |(_, label)| label)
}

/// How long until this pad's screen goes dark: zero once it's dark, and None
/// when it never does, because screen off is set to never or Mini Monitor is on.
pub fn time_left(cfg: &Config, kind: PadKind) -> Option<Duration> {
    let slot = slot(kind)?;
    if cfg.screen_off_ms == 0 || cfg.monitor_on(kind) {
        return None;
    }
    let mut last = lock(&LAST_PRESS);
    let since = *last[slot].get_or_insert_with(Instant::now);
    Some(Duration::from_millis(cfg.screen_off_ms).saturating_sub(since.elapsed()))
}

/// Whether this pad's screen should be dark now. For the 293S and the
/// AKP03E, whose drivers go by kind.
pub fn asleep(cfg: &Config, kind: PadKind) -> bool {
    let turned_off = kind.has_hardware() && cfg.pad_of_kind(kind).is_some_and(|entry| is_turned_off(&entry.id));
    turned_off || time_left(cfg, kind).is_some_and(|left| left.is_zero())
}

/// Whether a pad's screen is dark now, by its id: a virtual pad's only
/// when it was turned off.
pub fn pad_asleep(cfg: &Config, pad_id: &str) -> bool {
    match cfg.pad(pad_id) {
        Some(entry) if entry.kind.has_hardware() => asleep(cfg, entry.kind),
        Some(_) => is_turned_off(pad_id),
        None => false,
    }
}

fn is_turned_off(pad_id: &str) -> bool {
    lock(&TURNED_OFF).iter().any(|id| id == pad_id)
}

/// Turns a pad's screen off until it's turned on again or pressed, or back on.
pub fn set_screen(pad_id: &str, kind: PadKind, on: bool) {
    let mut off = lock(&TURNED_OFF);
    off.retain(|id| id != pad_id);
    if !on {
        off.push(pad_id.to_string());
    }
    drop(off);
    if on {
        touched(kind);
    }
}

/// A press on a pad: it counts as use, and brings back a screen that was turned off.
pub fn wake_pad(pad_id: &str, kind: PadKind) {
    lock(&TURNED_OFF).retain(|id| id != pad_id);
    touched(kind);
}

/// Counts a press, or a change that should be seen, as use of the pad.
pub fn touched(kind: PadKind) {
    if let Some(slot) = slot(kind) {
        lock(&LAST_PRESS)[slot] = Some(Instant::now());
    }
}

/// Turns a pad's screen off now, as if its time had run out. Only when screen
/// off is in use, so the next press wakes it as usual.
pub fn sleep_now(cfg: &Config, kind: PadKind) {
    let gone = Instant::now().checked_sub(Duration::from_millis(cfg.screen_off_ms));
    if let Some(slot) = slot(kind).filter(|_| cfg.screen_off_ms > 0 && gone.is_some()) {
        lock(&LAST_PRESS)[slot] = gone;
    }
}

/// A screen goes dark when its time runs out, with nothing else happening, so
/// this looks once a second and says when either pad's screen went dark or
/// came back. The drivers find out for themselves; this is for Home Assistant.
pub fn watch(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let mut last = [false; 2];
        loop {
            let now = {
                let cfg = lock(&shared.cfg);
                [asleep(&cfg, PadKind::Main), asleep(&cfg, PadKind::Akp03e)]
            };
            if now != last {
                last = now;
                shared.touch();
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// "4 min" or "1 h 20 min", for how long is left.
pub fn spoken(left: Duration) -> String {
    let minutes = left.as_secs().div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_goes_dark_only_after_its_time_and_never_in_mini_monitor() {
        let mut cfg = Config::default();
        assert!(!asleep(&cfg, PadKind::Akp03e), "never is the default");
        cfg.screen_off_ms = 60_000;
        lock(&LAST_PRESS)[1] = Some(Instant::now() - Duration::from_secs(61));
        assert!(asleep(&cfg, PadKind::Akp03e));
        touched(PadKind::Akp03e);
        assert!(!asleep(&cfg, PadKind::Akp03e));
        lock(&LAST_PRESS)[1] = Some(Instant::now() - Duration::from_secs(61));
        cfg.mini_monitor = true;
        cfg.monitor_pad = PadKind::Akp03e;
        cfg.add_small_pad("Small");
        assert!(!asleep(&cfg, PadKind::Akp03e));
    }

    #[test]
    fn a_pad_can_be_put_to_sleep_now_and_woken() {
        let mut cfg = Config::default();
        sleep_now(&cfg, PadKind::Main);
        assert!(!asleep(&cfg, PadKind::Main), "never means never");
        assert_eq!(time_left(&cfg, PadKind::Main), None);
        cfg.screen_off_ms = 300_000;
        touched(PadKind::Main);
        assert!(time_left(&cfg, PadKind::Main).is_some_and(|left| left > Duration::from_secs(290)));
        sleep_now(&cfg, PadKind::Main);
        assert!(asleep(&cfg, PadKind::Main));
        touched(PadKind::Main);
        assert!(!asleep(&cfg, PadKind::Main));
    }

    #[test]
    fn a_pad_turned_off_stays_dark_until_turned_on_or_pressed() {
        let mut cfg = Config::default();
        let wall = cfg.add_dashboard_pad("Wall", 2, 2);
        let main = cfg.pads[0].id.clone();
        assert!(!pad_asleep(&cfg, &wall));
        set_screen(&wall, PadKind::Dashboard, false);
        assert!(pad_asleep(&cfg, &wall));
        assert!(!pad_asleep(&cfg, &main), "only that pad");
        wake_pad(&wall, PadKind::Dashboard);
        assert!(!pad_asleep(&cfg, &wall));
        // The 293S goes dark even with screen off set to never, and its driver sees it.
        set_screen(&main, PadKind::Main, false);
        assert!(asleep(&cfg, PadKind::Main) && pad_asleep(&cfg, &main));
        set_screen(&main, PadKind::Main, true);
        assert!(!asleep(&cfg, PadKind::Main));
    }

    #[test]
    fn time_left_is_said_plainly() {
        assert_eq!(spoken(Duration::from_secs(10)), "1 min");
        assert_eq!(spoken(Duration::from_secs(240)), "4 min");
        assert_eq!(spoken(Duration::from_secs(3600)), "1 h");
        assert_eq!(spoken(Duration::from_secs(4800)), "1 h 20 min");
    }

    #[test]
    fn the_choices_are_the_originals() {
        let times: Vec<u64> = CHOICES.iter().map(|(ms, _)| *ms).collect();
        assert_eq!(times, [60_000, 300_000, 900_000, 1_800_000, 3_600_000, 7_200_000, 0]);
        assert_eq!(label_of(900_000), "After 15 minutes");
        assert_eq!(label_of(12_345), "Never");
    }
}
