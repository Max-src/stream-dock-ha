use super::*;

// The first pad of a kind, as the drivers see pads.
fn show_pad(cfg: &mut Config, kind: PadKind) {
    let id = cfg.pad_of_kind(kind).unwrap().id.clone();
    cfg.show_pad_id(&id);
}

fn profile_along(cfg: &Config, kind: PadKind, step: isize) -> Option<usize> {
    cfg.profile_along_pad(&cfg.pad_of_kind(kind)?.id, step)
}

fn adjust_brightness(cfg: &mut Config, kind: PadKind, change: i32) {
    let id = cfg.pad_of_kind(kind).unwrap().id.clone();
    cfg.adjust_pad_brightness(&id, change);
}

/// The 293S's brightness, then the AKP03E's.
fn levels(cfg: &Config) -> (u8, u8) {
    (cfg.brightness_of(PadKind::Main), cfg.brightness_of(PadKind::Akp03e))
}

fn level(from: f32, color: [u8; 3]) -> MeterLevel {
    MeterLevel { from, color }
}

#[test]
fn a_key_takes_the_colour_of_the_highest_number_reached() {
    let (blue, orange, red) = ([0, 0, 200], [250, 150, 0], [230, 0, 0]);
    let a = HaAction { level_colors: vec![level(26.0, orange), level(18.0, blue), level(30.0, red)], ..HaAction::default() };
    assert_eq!(a.color_for("12"), None);
    assert_eq!(a.color_for("18"), Some(blue));
    assert_eq!(a.color_for("25.9"), Some(blue));
    assert_eq!(a.color_for("26"), Some(orange));
    assert_eq!(a.color_for(" 41.5 "), Some(red));
    assert_eq!(a.color_for("-5"), None);
    assert_eq!(a.color_for("unavailable"), None);
}

#[test]
fn an_exact_state_comes_before_a_number() {
    let (blue, green) = ([0, 0, 200], [0, 200, 0]);
    let a = HaAction {
        level_colors: vec![level(0.0, blue)],
        state_colors: vec![StateColor { state: "20".into(), color: green, text: None }],
        ..HaAction::default()
    };
    assert_eq!(a.color_for("20"), Some(green));
    assert_eq!(a.color_for("21"), Some(blue));
}

#[test]
fn states_match_without_regard_to_capitals_and_empty_rows_match_nothing() {
    let green = [0, 200, 0];
    let rules = [StateColor::default(), StateColor { state: " Heating ".into(), color: green, text: None }];
    assert_eq!(color_of_state(&rules, "heating"), Some(green));
    assert_eq!(color_of_state(&rules, "cooling"), None);
    assert_eq!(color_of_state(&rules, ""), None);
}

#[test]
fn a_state_can_have_its_own_text_colour() {
    let (green, black) = ([0, 200, 0], [0, 0, 0]);
    let rules = [
        StateColor { state: "heating".into(), color: green, text: Some(black) },
        StateColor { state: "idle".into(), color: green, text: None },
    ];
    assert_eq!(text_color_of_state(&rules, "Heating"), Some(black));
    assert_eq!(text_color_of_state(&rules, "idle"), None);
    assert_eq!(text_color_of_state(&rules, "off"), None);
}

#[test]
fn keys_from_before_value_colours_still_load() {
    let a: HaAction = serde_json::from_str(r#"{"entity":"sensor.made_up"}"#).unwrap();
    assert!(a.state_colors.is_empty() && a.level_colors.is_empty());
    assert_eq!(a.color_for("20"), None);
}

#[test]
fn quitting_asks_first_unless_told_not_to() {
    assert!(Config::default().ask_before_quit);
    let cfg: Config = serde_json::from_str(r#"{"ask_before_quit": false}"#).unwrap();
    assert!(!cfg.ask_before_quit);
}

#[test]
fn settings_from_before_meters_keep_the_old_look() {
    let mut cfg: Config = serde_json::from_str(r#"{"band":[{"kind":"cpu"}]}"#).unwrap();
    cfg.repair();
    assert_eq!(cfg.band_meters.len(), 3);
    assert_eq!(cfg.band_meters[0], Meter::default());
    assert_eq!(cfg.band_meters[0].style, MeterStyle::Bar);
    assert_eq!(cfg.band_meters[0].percent_levels(), [(0, [79, 140, 255]), (90, [226, 92, 92])]);
}

#[test]
fn presets_follow_the_range_and_are_recognised() {
    let mut meter = Meter { low: 10.0, high: 30.0, ..Meter::default() };
    meter.use_preset(1);
    let starts: Vec<f32> = meter.levels.iter().map(|level| level.from.round()).collect();
    assert_eq!(starts, [10.0, 22.0, 27.0]);
    assert_eq!(meter.preset(), Some(1));
    meter.levels[1].from = 20.0;
    assert_eq!(meter.preset(), None);
}

#[test]
fn a_broken_meter_is_put_right() {
    let level = |from, color| MeterLevel { from, color };
    let mut meter = Meter {
        levels: vec![level(70.0, [1, 1, 1]), level(250.0, [2, 2, 2]), level(20.0, [3, 3, 3])],
        ..Meter::default()
    };
    meter.repair();
    assert_eq!(meter.levels, [level(0.0, [3, 3, 3]), level(70.0, [1, 1, 1]), level(100.0, [2, 2, 2])]);

    let mut empty = Meter { levels: Vec::new(), low: 5.0, high: 5.0, ..Meter::default() };
    empty.repair();
    assert!(empty.high > empty.low && !empty.levels.is_empty());
}

#[test]
fn levels_are_added_up_to_the_limit() {
    let mut meter = Meter::default();
    meter.use_preset(4);
    for _ in 0..10 {
        meter.add_level();
    }
    assert_eq!(meter.levels.len(), MAX_METER_LEVELS);
    assert!(meter.levels.windows(2).all(|pair| pair[0].from < pair[1].from));
}

#[test]
fn a_reading_stays_inside_its_range() {
    let meter = Meter { low: 15.0, high: 35.0, ..Meter::default() };
    assert_eq!(meter.percent_of(25.0), 50);
    assert_eq!(meter.percent_of(-40.0), 0);
    assert_eq!(meter.percent_of(90.0), 100);
}

#[test]
fn settings_from_before_the_second_pad_still_load() {
    let saved = r#"{"profiles":[{"name":"Mine","rows":3,"cols":5}],"active_profile":0}"#;
    let cfg: Config = serde_json::from_str(saved).unwrap();
    assert_eq!(cfg.profiles[0].pad, PadKind::Main);
    assert_eq!(cfg.main_index(), 0);
    assert_eq!(cfg.small_pad_index(), None);
}

#[test]
fn the_main_pad_keeps_its_profile_while_the_window_shows_the_small_pad() {
    let mut cfg = Config::default();
    cfg.profiles.push(Profile { name: "Second".into(), ..Profile::default() });
    cfg.choose_profile(1);
    cfg.add_small_pad("Small");
    assert_eq!(cfg.active_profile, 2);
    assert_eq!(cfg.main_index(), 1);
    cfg.repair();
    assert_eq!(cfg.main_profile().name, "Second");
    assert_eq!(cfg.small_pad_index(), Some(2));
}

#[test]
fn deleting_an_earlier_profile_keeps_the_main_pad_on_the_same_one() {
    let mut cfg = Config::default();
    cfg.add_small_pad("Small");
    cfg.profiles.swap(0, 1);
    cfg.active_profile = 0;
    cfg.entry_of_kind_mut(PadKind::Main).unwrap().last_profile = 1;
    cfg.remove_active_profile();
    cfg.repair();
    assert_eq!(cfg.profiles.len(), 1);
    assert_eq!((cfg.pad_of_kind(PadKind::Main).unwrap().last_profile, cfg.main_index()), (0, 0));
}

/// Main 0, Main 1, then the small pad's 2 and 3.
fn both_pads() -> Config {
    let mut cfg = Config::default();
    cfg.profiles.push(Profile { name: "Second".into(), ..Profile::default() });
    cfg.add_small_pad("Small");
    cfg.add_small_pad("Small");
    cfg
}

#[test]
fn each_pad_keeps_its_own_profile_while_the_window_moves_between_them() {
    let mut cfg = both_pads();
    cfg.choose_profile(1);
    cfg.choose_profile(3);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(3)));
    show_pad(&mut cfg, PadKind::Main);
    assert_eq!(cfg.active_profile, 1);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(3)));
    show_pad(&mut cfg, PadKind::Akp03e);
    assert_eq!(cfg.active_profile, 3);
}

#[test]
fn choosing_a_profile_for_one_pad_leaves_the_other_alone() {
    let mut cfg = both_pads();
    cfg.choose_profile(2);
    cfg.choose_profile(1);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(2)));
    cfg.choose_profile(0);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (0, Some(2)));
}

#[test]
fn a_key_can_switch_a_pad_without_moving_the_window_off_the_other_pad() {
    let mut cfg = both_pads();
    cfg.choose_profile(2);
    cfg.run_profile(1);
    assert_eq!(cfg.active_profile, 2);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(2)));
    cfg.run_profile(3);
    assert_eq!(cfg.active_profile, 3);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(3)));
}

#[test]
fn each_pad_keeps_its_own_brightness() {
    let mut cfg = Config::default();
    cfg.add_small_pad("Small");
    assert_eq!(levels(&cfg), (25, 25));
    adjust_brightness(&mut cfg, PadKind::Akp03e, 5);
    assert_eq!(levels(&cfg), (25, 30));
    adjust_brightness(&mut cfg, PadKind::Main, -5);
    assert_eq!(levels(&cfg), (20, 30));
}

#[test]
fn brightness_steps_stay_within_the_pads_range() {
    let mut cfg = Config::default();
    cfg.add_small_pad("Small");
    cfg.pads[0].brightness = 98;
    cfg.pads[1].brightness = 3;
    adjust_brightness(&mut cfg, PadKind::Main, 5);
    adjust_brightness(&mut cfg, PadKind::Akp03e, -5);
    assert_eq!(levels(&cfg), (100, 0));
}

#[test]
fn saved_settings_keep_their_brightness() {
    let mut cfg: Config = serde_json::from_str(r#"{"brightness":70}"#).unwrap();
    cfg.repair();
    assert_eq!(levels(&cfg), (70, 25));
}

#[test]
fn stepping_through_profiles_stays_on_the_pad_and_wraps_both_ways() {
    let mut cfg = both_pads();
    cfg.choose_profile(1);
    cfg.choose_profile(2);
    assert_eq!(profile_along(&cfg, PadKind::Akp03e, 1), Some(3));
    assert_eq!(profile_along(&cfg, PadKind::Akp03e, -1), Some(3));
    cfg.choose_profile(3);
    assert_eq!(profile_along(&cfg, PadKind::Akp03e, 1), Some(2));
    assert_eq!(profile_along(&cfg, PadKind::Main, 1), Some(0));
    assert_eq!(profile_along(&cfg, PadKind::Main, -1), Some(0));
}

#[test]
fn stepping_with_one_profile_stays_put() {
    let mut cfg = Config::default();
    assert_eq!(profile_along(&cfg, PadKind::Main, 1), Some(0));
    assert_eq!(profile_along(&cfg, PadKind::Akp03e, 1), None);
    cfg.add_small_pad("Small");
    assert_eq!(profile_along(&cfg, PadKind::Akp03e, -1), Some(1));
}

#[test]
fn every_knob_owns_its_three_cells() {
    for knob in 0..3 {
        for cell in small_knob_cells(knob) {
            assert_eq!(small_knob_of_cell(cell), Some(knob));
        }
    }
    for cell in [0, 1, 2, 6, 7, 8, 12, 13, 14, 18] {
        assert_eq!(small_knob_of_cell(cell), None, "cell {cell}");
    }
}

#[test]
fn mini_monitor_runs_on_the_chosen_pad_only() {
    let mut cfg = both_pads();
    cfg.mini_monitor = true;
    assert!(cfg.monitor_on(PadKind::Main) && !cfg.monitor_on(PadKind::Akp03e));
    cfg.monitor_pad = PadKind::Akp03e;
    assert!(!cfg.monitor_on(PadKind::Main) && cfg.monitor_on(PadKind::Akp03e));
}

#[test]
fn mini_monitor_falls_back_when_its_pad_has_no_profile() {
    let mut cfg = Config { monitor_pad: PadKind::Akp03e, ..Config::default() };
    cfg.repair();
    assert_eq!(cfg.monitor_pad, PadKind::Main);
}

fn camera_key(span: [u8; 2]) -> DockKey {
    let binding = HaAction { entity: "camera.door".into(), ..HaAction::default() };
    DockKey { actions: vec![Action::HomeAssistant(binding)], span, ..DockKey::default() }
}

#[test]
fn a_stretched_camera_covers_its_keys_only() {
    let mut profile = Profile::default();
    profile.pages[0].keys[1] = camera_key([2, 2]);
    let cover = |cell: usize| profile.cover(0, cell);
    assert_eq!(cover(1), Some(Cover { owner: 1, at: [0, 0], span: [2, 2] }));
    assert_eq!(cover(2), Some(Cover { owner: 1, at: [1, 0], span: [2, 2] }));
    assert_eq!(cover(6), Some(Cover { owner: 1, at: [0, 1], span: [2, 2] }));
    assert_eq!(cover(7), Some(Cover { owner: 1, at: [1, 1], span: [2, 2] }));
    for cell in [0, 3, 4, 5, 8, 10, 11, 14] {
        assert_eq!(cover(cell), None, "cell {cell}");
    }
    assert_eq!(profile.owner_of(0, 7), 1);
    assert_eq!(profile.owner_of(0, 8), 8);
}

#[test]
fn a_camera_stays_inside_the_screen_keys() {
    let mut profile = Profile::default();
    assert_eq!(profile.largest_span(13), [2, 1]);
    profile.pages[0].keys[13] = camera_key([9, 9]);
    assert_eq!(profile.cover(0, 14), Some(Cover { owner: 13, at: [1, 0], span: [2, 1] }));

    let mut small = Profile::for_small_pad("Small".into());
    assert_eq!(small.largest_span(0), [3, 2]);
    assert_eq!(small.largest_span(7), [2, 1]);
    assert_eq!(small.largest_span(12), [1, 1]);
    assert_eq!(small.largest_span(4), [1, 1]);
    small.pages[0].keys[0] = camera_key([3, 2]);
    assert_eq!(small.cover(0, 8), Some(Cover { owner: 0, at: [2, 1], span: [3, 2] }));
    assert_eq!(small.cover(0, 3), None);
    assert_eq!(small.cover(0, 12), None);
}

#[test]
fn a_copy_has_everything_but_the_shortcut() {
    let key = DockKey { title: "Mic".into(), hotkey: "Ctrl+Alt+1".into(), ..DockKey::default() };
    let copy = key.copied();
    assert_eq!(copy.title, "Mic");
    assert!(copy.hotkey.is_empty());
}

#[test]
fn the_next_free_key_goes_round_the_page() {
    let mut profile = Profile::default();
    for cell in [0, 1, 3] {
        profile.pages[0].keys[cell].title = "Taken".into();
    }
    assert_eq!(profile.free_cell(0, 1), Some(2));
    assert_eq!(profile.free_cell(0, 3), Some(4));
    for cell in 2..15 {
        profile.pages[0].keys[cell].title = "Taken".into();
    }
    profile.pages[0].keys[0] = DockKey::default();
    assert_eq!(profile.free_cell(0, 9), Some(0));
    profile.pages[0].keys[0].title = "Taken".into();
    assert_eq!(profile.free_cell(0, 9), None);
}

#[test]
fn a_copy_on_the_small_pad_goes_to_a_key_with_a_screen() {
    let mut small = Profile::for_small_pad("Small".into());
    small.pages[0].keys[0].title = "Taken".into();
    small.pages[0].keys[3] = DockKey::default();
    assert_eq!(small.free_cell(0, 0), Some(1));
    for cell in [1, 2, 6, 7, 8] {
        small.pages[0].keys[cell].title = "Taken".into();
    }
    assert_eq!(small.free_cell(0, 0), None);
}

#[test]
fn a_stretched_camera_leaves_no_free_key_under_it() {
    let mut profile = Profile::default();
    profile.pages[0].keys[0] = camera_key([2, 1]);
    assert_eq!(profile.free_cell(0, 0), Some(2));
}

#[test]
fn deleting_asks_first_until_told_not_to() {
    assert!(Config::default().ask_before_delete);
    let saved: Config = serde_json::from_str(r#"{"ask_before_delete":false}"#).unwrap();
    assert!(!saved.ask_before_delete);
    let older: Config = serde_json::from_str("{}").unwrap();
    assert!(older.ask_before_delete);
}

fn titled(profile: &mut Profile) {
    for (cell, key) in profile.pages[0].keys.iter_mut().enumerate() {
        key.title = cell.to_string();
    }
}

fn titles(profile: &Profile) -> Vec<String> {
    profile.pages[0].keys.iter().map(|key| key.title.clone()).collect()
}

#[test]
fn a_stretched_camera_swaps_places_with_the_keys_it_lands_on() {
    let mut profile = Profile::default();
    titled(&mut profile);
    profile.pages[0].keys[0] = camera_key([2, 2]);
    // Cells 0, 1, 5, 6 go to 2, 3, 7, 8, and those keys come back.
    let moves = profile.block_moves(0, 0, [2, 2], 2).unwrap();
    apply_moves(&mut profile.pages[0].keys, &moves);
    let cells = titles(&profile);
    assert_eq!(profile.cover(0, 3), Some(Cover { owner: 2, at: [1, 0], span: [2, 2] }));
    assert_eq!(cells[0], "2");
    assert_eq!(cells[1], "3");
    assert_eq!(cells[5], "7");
    assert_eq!(cells[6], "8");
    // What was under the camera is still there, under it.
    assert_eq!(cells[3], "1");
    assert_eq!(cells[7], "5");
    assert_eq!(cells[8], "6");
}

#[test]
fn a_small_move_only_shifts_the_keys_it_needs_to() {
    let mut profile = Profile::default();
    titled(&mut profile);
    profile.pages[0].keys[0] = camera_key([2, 1]);
    let moves = profile.block_moves(0, 0, [2, 1], 1).unwrap();
    apply_moves(&mut profile.pages[0].keys, &moves);
    let cells = titles(&profile);
    assert_eq!(profile.cover(0, 2), Some(Cover { owner: 1, at: [1, 0], span: [2, 1] }));
    assert_eq!(cells[0], "2");
    assert_eq!(cells[2], "1");
}

#[test]
fn no_key_is_lost_or_doubled_by_a_block_move() {
    for to in [1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13] {
        let mut profile = Profile::default();
        titled(&mut profile);
        let before = titles(&profile);
        profile.pages[0].keys[0].actions = camera_key([2, 1]).actions;
        profile.pages[0].keys[0].span = [2, 1];
        let moves = profile.block_moves(0, 0, [2, 1], to).unwrap();
        apply_moves(&mut profile.pages[0].keys, &moves);
        let mut after = titles(&profile);
        after.sort();
        let mut expected = before;
        expected.sort();
        assert_eq!(after, expected, "to {to}");
    }
}

#[test]
fn a_block_stays_on_the_pad() {
    let mut profile = Profile::default();
    profile.pages[0].keys[0] = camera_key([3, 2]);
    assert_eq!(profile.block_home(14, [3, 2]), 7);
    assert!(profile.block_moves(0, 0, [3, 2], 13).is_none());
    let small = Profile::for_small_pad("Small".into());
    assert_eq!(small.block_home(8, [2, 2]), 1);
}

#[test]
fn a_block_cant_land_on_another_camera() {
    let mut profile = Profile::default();
    profile.pages[0].keys[0] = camera_key([2, 1]);
    profile.pages[0].keys[2] = camera_key([2, 1]);
    assert!(profile.block_moves(0, 0, [2, 1], 1).is_none());
    assert!(profile.block_moves(0, 0, [2, 1], 5).is_some());
}

#[test]
fn control_is_off_until_turned_on() {
    let cfg = Config::default();
    assert!(!cfg.ha_control && !cfg.ha_control_programs);
}

#[test]
fn typing_steps_saved_before_the_enter_option_still_load() {
    let step: Action = serde_json::from_str(r#"{"kind":"type_text","text":"hello"}"#).unwrap();
    assert_eq!(step, Action::TypeText { text: "hello".into(), press_enter: false });
}

#[test]
fn only_cameras_stretch() {
    let mut profile = Profile::default();
    profile.pages[0].keys[0].span = [3, 3];
    profile.pages[0].keys[0].title = "Not a camera".into();
    assert_eq!(profile.cover(0, 1), None);
}

#[test]
fn keys_saved_before_stretching_are_one_key_big() {
    let key: DockKey = serde_json::from_str(r#"{"title":"Old"}"#).unwrap();
    assert_eq!(key.span, [1, 1]);
}

#[test]
fn a_new_profile_is_for_the_pad_in_the_window() {
    let mut cfg = Config::default();
    cfg.add_small_pad("Small");
    cfg.add_profile();
    assert_eq!(cfg.profile().pad, PadKind::Akp03e);
    assert_eq!(cfg.profile().pages[0].keys.len(), 18);
    show_pad(&mut cfg, PadKind::Main);
    cfg.add_profile();
    assert_eq!(cfg.profile().pad, PadKind::Main);
}

#[test]
fn deleting_a_profile_keeps_each_pad_on_what_it_runs() {
    let mut cfg = both_pads();
    cfg.choose_profile(1);
    cfg.choose_profile(3);
    cfg.remove_active_profile();
    assert_eq!(cfg.profile().pad, PadKind::Akp03e);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, Some(2)));
    cfg.remove_active_profile();
    assert_eq!(cfg.profile().pad, PadKind::Main);
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (1, None));
}

#[test]
fn every_page_of_the_small_pad_has_its_controls_labelled() {
    let mut profile = Profile::for_small_pad("Small".into());
    profile.add_page();
    assert_eq!((profile.rows, profile.cols), (3, 6));
    for page in &profile.pages {
        assert_eq!(page.keys.len(), 18);
        for (cell, key) in page.keys.iter().enumerate() {
            assert_eq!(key.title.is_empty(), small_cell_has_screen(cell), "cell {cell}");
            if !small_cell_has_screen(cell) {
                assert!(crate::icons::character(&key.glyph).is_some(), "{}", key.glyph);
            }
        }
    }
}

#[test]
fn the_small_pad_starts_with_the_originals_rules() {
    let keys = &Profile::for_small_pad("Small".into()).pages[0].keys;
    let step = |cell: usize| keys[cell].actions.first();
    assert_eq!(step(12), Some(&Action::PreviousProfile));
    assert_eq!(step(13), Some(&Action::FirstProfile));
    assert_eq!(step(14), Some(&Action::NextProfile));
    assert_eq!(step(4), Some(&Action::Media { key: MediaKey::VolumeDown }));
    assert_eq!(step(10), Some(&Action::Media { key: MediaKey::VolumeUp }));
    assert_eq!(step(16), Some(&Action::Media { key: MediaKey::Mute }));
    assert_eq!(step(3), Some(&Action::PreviousPage));
    assert_eq!(step(9), Some(&Action::NextPage));
    assert_eq!(step(15), Some(&Action::GoToPage { page: 1 }));
    assert_eq!(step(5), Some(&Action::AdjustBrightness { change: -5 }));
    assert_eq!(step(11), Some(&Action::AdjustBrightness { change: 5 }));
    assert_eq!(step(17), Some(&Action::Media { key: MediaKey::MicMute }));
    assert_eq!(KnobUse::of(keys, 0), Some(KnobUse::Pages));
    assert_eq!(KnobUse::of(keys, 1), Some(KnobUse::Volume));
    // Brightness turns with a microphone press is no single use.
    assert_eq!(KnobUse::of(keys, 2), None);
}

#[test]
fn a_whole_knob_is_set_in_one_go_and_recognised() {
    let mut page = Profile::small_pad_page();
    for knob in 0..3 {
        for choice in KnobUse::ALL {
            page.set_knob(knob, choice);
            assert_eq!(KnobUse::of(&page.keys, knob), Some(choice), "knob {knob}, {choice:?}");
            for cell in small_knob_cells(knob) {
                let key = &page.keys[cell];
                assert!(key.title.starts_with(SMALL_KNOB_SHORT[knob]), "{}", key.title);
                assert!(crate::icons::character(&key.glyph).is_some(), "{}", key.glyph);
            }
        }
    }
    page.set_knob(2, KnobUse::Microphone);
    let [left, right, press] = small_knob_cells(2).map(|cell| page.keys[cell].actions.clone());
    assert_eq!(left, [Action::Media { key: MediaKey::MicDown }]);
    assert_eq!(right, [Action::Media { key: MediaKey::MicUp }]);
    assert_eq!(press, [Action::Media { key: MediaKey::MicMute }]);
    // A second step makes it the person's own mix.
    page.keys[small_knob_cells(2)[2]].actions.push(Action::Wait { ms: 10 });
    assert_eq!(KnobUse::of(&page.keys, 2), None);
}

#[test]
fn the_screen_keys_are_the_top_left_of_the_small_pad() {
    let cells: Vec<usize> = (0..SMALL_SCREEN_KEYS).map(small_cell_of_key).collect();
    assert_eq!(cells, [0, 1, 2, 6, 7, 8]);
}

#[test]
fn titles_on_the_small_pad_come_out_the_same_size() {
    let style = KeyStyle::default();
    let profile = Profile::for_small_pad("Small".into());
    let drawn = profile.style_on_pad(&style).size as f32 * 60.0 / 85.0;
    assert!((drawn - style.size as f32).abs() < 1.0);
    assert_eq!(Profile::default().style_on_pad(&style).size, style.size);
}

#[test]
fn saved_setup_reads_back_the_same() {
    let mut cfg = Config::sample();
    cfg.profiles[0].pages[0].keys[0].hotkey = "Ctrl+Alt+1".into();
    let text = serde_json::to_string(&cfg).unwrap();
    let back: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(cfg, back);
}

#[test]
fn shrinking_the_grid_keeps_the_keys() {
    let mut p = Config::sample().profiles.remove(0);
    p.rows = 2;
    p.cols = 3;
    p.repair();
    assert_eq!(p.pages[0].keys[12].title, "Snip");
}

#[test]
fn a_saved_profile_opens_again_unchanged() {
    let path = std::env::temp_dir().join("stream-dock-ha-profile-test.json");
    let profile = Config::sample().profiles.remove(0);
    export_profile(&profile, &path).unwrap();
    assert_eq!(import_profile(&path).unwrap(), profile);
}

#[test]
fn an_opened_profile_never_lets_home_assistant_press_more_than_automatic() {
    let path = std::env::temp_dir().join("stream-dock-ha-profile-remote-test.json");
    let mut profile = Config::sample().profiles.remove(0);
    profile.pages[0].keys[0].remote = RemoteAccess::Allow;
    profile.pages[0].keys[1].remote = RemoteAccess::Deny;
    profile.pages.push(Page::default());
    profile.repair();
    profile.pages[1].keys[2].remote = RemoteAccess::Allow;
    export_profile(&profile, &path).unwrap();
    let opened = import_profile(&path).unwrap();
    assert_eq!(opened.pages[0].keys[0].remote, RemoteAccess::Auto);
    assert_eq!(opened.pages[0].keys[1].remote, RemoteAccess::Deny, "\"Never\" stays");
    assert_eq!(opened.pages[1].keys[2].remote, RemoteAccess::Auto);
}

#[test]
fn a_profile_saved_under_the_old_name_still_opens() {
    let path = std::env::temp_dir().join("stream-dock-ha-old-profile-test.json");
    let profile = Config::sample().profiles.remove(0);
    for format in ["stream-dock-max-profile", "tray-dock-profile"] {
        let file = serde_json::json!({ "format": format, "version": 1, "profile": profile });
        std::fs::write(&path, file.to_string()).unwrap();
        assert_eq!(import_profile(&path).unwrap(), profile);
    }
}

#[test]
fn settings_from_the_old_name_are_copied_once_and_left_in_place() {
    let dir = std::env::temp_dir().join("stream-dock-ha-copy-test");
    let _ = std::fs::remove_dir_all(&dir);
    let (old, new) = (dir.join("TrayDock").join("config.json"), dir.join("StreamDockHA").join("config.json"));
    assert!(!copy_settings(&old, &new));
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "old").unwrap();
    assert!(copy_settings(&old, &new));
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
    std::fs::write(&new, "newer").unwrap();
    assert!(!copy_settings(&old, &new));
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "newer");
}

#[test]
fn a_profile_from_the_original_app_is_named_as_such() {
    for name in ["work.streamDockProfile", "Work.STREAMDOCKPROFILE", "deck.monstardeckProfile"] {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, b"PK").unwrap();
        let problem = import_profile(&path).unwrap_err();
        assert!(problem.contains("StreamDock app"), "{name}: {problem}");
    }
}

#[test]
fn new_keys_on_the_akp03e_go_on_screen_keys_only() {
    let mut profile = Profile::for_small_pad("Desk".into());
    for cell in [0, 1, 2] {
        profile.pages[0].keys[cell].title = "taken".into();
    }
    assert_eq!(profile.first_free_cell(0), Some(6));
    let empty = Profile::default();
    assert_eq!(empty.first_free_cell(0), Some(0));
}

#[test]
fn an_older_copy_of_the_settings_is_never_saved_over_a_newer_one() {
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-save-order-test.json"));
    let (shared, _opened) = Shared::new(Config::default(), false);
    *lock(&shared.saved) = 5;
    shared.write(&Config::default(), 3);
    assert_eq!(*lock(&shared.saved), 5);
    shared.write(&Config::default(), 6);
    assert_eq!(*lock(&shared.saved), 6);
}

#[test]
fn other_files_are_refused() {
    let path = std::env::temp_dir().join("stream-dock-ha-not-a-profile.json");
    std::fs::write(&path, r#"{"hello":"world"}"#).unwrap();
    assert!(import_profile(&path).is_err());
    std::fs::write(&path, r#"{"format":"something-else","version":1,"profile":{}}"#).unwrap();
    assert!(import_profile(&path).is_err());
}

#[test]
fn copies_get_a_free_name() {
    let mut cfg = Config::sample();
    assert_eq!(cfg.free_name("Work"), "Work");
    assert_eq!(cfg.free_name("Everyday"), "Everyday 2");
    cfg.profiles.push(Profile { name: "Everyday 2".into(), ..Profile::default() });
    assert_eq!(cfg.free_name("Everyday"), "Everyday 3");
}

#[test]
fn damaged_values_are_repaired() {
    let mut cfg: Config =
        serde_json::from_str(r#"{"profiles":[{"rows":0,"cols":99,"pages":[],"active_page":7}],"active_profile":4}"#)
            .unwrap();
    cfg.repair();
    assert_eq!(cfg.active_profile, 0);
    assert_eq!(cfg.profile().active_page, 0);
    assert_eq!(cfg.profile().pages[0].keys.len(), 10);
}

#[test]
fn a_settings_file_that_cant_be_read_says_where() {
    let why = super::storage::parse_settings(b"{ \"profiles\": [ }".to_vec()).unwrap_err();
    assert!(why.contains("line 1"), "{why}");
    assert_eq!(super::storage::parse_settings(vec![0xff, 0xfe]).unwrap_err(), "the file isn't text");
    assert!(super::storage::parse_settings(serde_json::to_vec(&Config::default()).unwrap()).is_ok());
}

/// Settings saved before pads had ids: both pads' own fields at the top level,
/// and profiles that only say which kind of pad they're for.
const OLD_SETTINGS: &str = r#"{
    "profiles": [
        {"name": "Desk", "rows": 3, "cols": 5},
        {"name": "Small", "pad": "akp03e", "rows": 3, "cols": 6},
        {"name": "Desk 2", "pad": "main"}
    ],
    "active_profile": 2,
    "last_main_profile": 0,
    "last_small_profile": 1,
    "brightness": 70,
    "small_brightness": 40,
    "gap_across": 30,
    "gap_down": 31,
    "small_gap_across": 20,
    "small_gap_down": 21,
    "main_face": [10, 20, 30],
    "small_face": [1, 2, 3],
    "main_face_clear": true,
    "small_face_clear": false,
    "monitor_pad": "akp03e"
}"#;

fn migrated() -> Config {
    let mut cfg = super::storage::parse_settings(OLD_SETTINGS.as_bytes().to_vec()).unwrap();
    cfg.repair();
    cfg
}

#[test]
fn old_settings_become_one_pad_of_each_kind() {
    let cfg = migrated();
    assert_eq!(cfg.pads.len(), 2);
    let (main, small) = (&cfg.pads[0], &cfg.pads[1]);
    assert_eq!((main.kind, main.name.as_str()), (PadKind::Main, "Stream Dock 293S"));
    assert_eq!((main.brightness, main.gaps, main.face, main.face_clear, main.last_profile), (70, [30, 31], [10, 20, 30], true, 0));
    assert_eq!((small.kind, small.name.as_str()), (PadKind::Akp03e, "AJAZZ AKP03E"));
    assert_eq!((small.brightness, small.gaps, small.face, small.face_clear, small.last_profile), (40, [20, 21], [1, 2, 3], false, 1));
    assert!(is_id(&main.id) && is_id(&small.id) && main.id != small.id);
    let owners: Vec<&str> = cfg.profiles.iter().map(|p| p.pad_id.as_str()).collect();
    assert_eq!(owners, [main.id.as_str(), small.id.as_str(), main.id.as_str()]);
    assert!(cfg.profiles.iter().all(|p| is_id(&p.id)));
    // Each pad still runs what it ran: the window's profile, and the AKP03E's last one.
    assert_eq!((cfg.main_index(), cfg.small_pad_index()), (2, Some(1)));
    assert_eq!(cfg.monitor_pad, PadKind::Akp03e);
    assert_eq!(cfg.profiles_of(&main.id), [0, 2]);
}

#[test]
fn old_settings_without_an_akp03e_get_only_the_293s() {
    let saved = r#"{"profiles":[{"name":"Mine"}],"small_brightness":60,"small_gap_across":10,"brightness":45}"#;
    let mut cfg: Config = serde_json::from_str(saved).unwrap();
    cfg.repair();
    assert_eq!(cfg.pads.len(), 1);
    assert_eq!((cfg.pads[0].kind, cfg.pads[0].brightness), (PadKind::Main, 45));
    assert_eq!(cfg.profiles[0].pad_id, cfg.pads[0].id);
    // Settings with nothing in them at all still get their 293S.
    let mut empty: Config = serde_json::from_str("{}").unwrap();
    empty.repair();
    assert_eq!(empty.pads.len(), 1);
    assert_eq!(empty.profiles[0].pad_id, empty.pads[0].id);
}

#[test]
fn the_old_fields_are_read_but_not_written() {
    let cfg = migrated();
    let saved = serde_json::to_value(&cfg).unwrap();
    for old in [
        "brightness", "small_brightness", "gap_across", "gap_down", "small_gap_across", "small_gap_down",
        "main_face", "small_face", "main_face_clear", "small_face_clear", "last_main_profile", "last_small_profile",
    ] {
        assert!(saved.get(old).is_none(), "{old} is still written");
    }
    assert_eq!(saved["pads"][1]["kind"], "akp03e");
    assert_eq!(saved["profiles"][1]["pad_id"], saved["pads"][1]["id"]);
    // Read back, it's the same, ids and all.
    let mut back: Config = serde_json::from_value(saved).unwrap();
    back.repair();
    assert_eq!(back.pads, cfg.pads);
    assert_eq!(back.profiles, cfg.profiles);
}

#[test]
fn repairing_twice_changes_nothing_more() {
    let mut cfg = migrated();
    cfg.add_dashboard_pad("Wall", 2, 4);
    cfg.repair();
    let once = cfg.clone();
    cfg.repair();
    assert_eq!(cfg, once);
    let mut fresh = Config::default();
    let before = fresh.clone();
    fresh.repair();
    assert_eq!(fresh, before);
}

#[test]
fn copied_ids_are_made_unique() {
    let mut cfg = Config::default();
    let first = cfg.add_dashboard_pad("Wall", 2, 4);
    cfg.add_dashboard_pad("Kitchen", 1, 3);
    // As if the file had been edited by hand: the same ids twice.
    cfg.pads[2].id = first.clone();
    cfg.profiles[2].pad_id = first.clone();
    cfg.profiles[2].id = cfg.profiles[1].id.clone();
    // And a second 293S, which the driver couldn't run.
    let mut extra = PadEntry::new(PadKind::Main);
    extra.name = "Second 293S".into();
    cfg.profiles.push(Profile { id: new_id(), pad_id: extra.id.clone(), name: "Extra".into(), ..Profile::default() });
    cfg.pads.push(extra);
    cfg.repair();
    let ids: std::collections::HashSet<&str> = cfg.pads.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids.len(), cfg.pads.len());
    let profile_ids: std::collections::HashSet<&str> = cfg.profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(profile_ids.len(), cfg.profiles.len());
    assert_eq!(cfg.pads.iter().filter(|entry| entry.kind == PadKind::Main).count(), 1);
    let extra = cfg.profiles.iter().find(|p| p.name == "Extra").unwrap();
    assert_eq!(extra.pad_id, cfg.pads[0].id, "the extra 293S's profile moves to the first");
    // Every pad has a profile of its own kind, and every profile a pad.
    for entry in &cfg.pads {
        let mine = cfg.profiles_of(&entry.id);
        assert!(!mine.is_empty(), "{} has no profile", entry.name);
        assert!(mine.iter().all(|i| cfg.profiles[*i].pad == entry.kind));
    }
    assert!(cfg.profiles.iter().all(|p| cfg.pad(&p.pad_id).is_some()));
}

#[test]
fn a_profile_follows_its_own_kind_of_pad() {
    let mut cfg = Config::default();
    let main = cfg.pads[0].id.clone();
    cfg.profiles.push(Profile { pad_id: main.clone(), ..Profile::for_small_pad("Small".into()) });
    cfg.repair();
    let small = cfg.pad_of_kind(PadKind::Akp03e).expect("an AKP03E is set up for it");
    assert_eq!(cfg.profiles[1].pad_id, small.id);
    assert_ne!(small.id, main);
}

#[test]
fn an_akp03e_with_no_profile_is_not_in_use_and_a_dashboard_pad_keeps_one() {
    let mut cfg = Config::default();
    cfg.pads.push(PadEntry::new(PadKind::Akp03e));
    let wall = PadEntry::new(PadKind::Dashboard);
    let wall_id = wall.id.clone();
    cfg.pads.push(wall);
    cfg.repair();
    assert!(cfg.pad_of_kind(PadKind::Akp03e).is_none());
    let mine = cfg.profiles_of(&wall_id);
    assert_eq!(mine.len(), 1);
    assert_eq!(cfg.profiles[mine[0]].pad, PadKind::Dashboard);
}

#[test]
fn the_akp03e_keeps_its_settings_while_it_has_no_profiles() {
    let mut cfg = Config::default();
    cfg.add_small_pad("Small");
    let entry = cfg.pad_of_kind(PadKind::Akp03e).unwrap();
    let id = entry.id.clone();
    {
        let small = cfg.pad_mut(&id).unwrap();
        small.brightness = 70;
        small.gaps = [12, 34];
        small.face = [1, 2, 3];
    }
    cfg.remove_active_profile();
    assert!(cfg.pad_of_kind(PadKind::Akp03e).is_none(), "it isn't in use");
    assert!(!cfg.in_use(&id));
    assert!(cfg.pads_in_use().all(|entry| entry.id != id));
    assert_eq!(cfg.brightness_of(PadKind::Akp03e), DEFAULT_BRIGHTNESS, "as when it wasn't in use");
    // Saved and opened again.
    let mut cfg: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
    cfg.repair();
    assert!(cfg.profiles.iter().all(|profile| profile.pad != PadKind::Akp03e && profile.pad_id != id));
    cfg.add_small_pad("Small again");
    let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap();
    assert_eq!((small.id.as_str(), small.brightness, small.gaps, small.face), (id.as_str(), 70, [12, 34], [1, 2, 3]));
    assert_eq!(cfg.pads.iter().filter(|entry| entry.kind == PadKind::Akp03e).count(), 1);
}

/// How many profiles each pad of `OLD_SETTINGS` has.
fn migrated_count(kind: PadKind) -> usize {
    match kind {
        PadKind::Main => 2,
        PadKind::Akp03e => 1,
        PadKind::Dashboard => 0,
    }
}

#[test]
fn a_new_pc_gets_a_new_instance_and_new_pad_ids() {
    let mut cfg = migrated();
    let (here, elsewhere) = (machine_hash_of("this pc"), machine_hash_of("another pc"));
    assert!(cfg.adopt_machine(&here));
    assert!(is_id(&cfg.instance_id));
    assert_eq!(cfg.instance_machine, here);
    let (instance, pads) = (cfg.instance_id.clone(), cfg.pads.clone());
    assert!(!cfg.adopt_machine(&here), "the same PC keeps its ids");
    assert_eq!((&cfg.instance_id, &cfg.pads), (&instance, &pads));
    // The settings were copied to another PC.
    assert!(cfg.adopt_machine(&elsewhere));
    assert_ne!(cfg.instance_id, instance);
    assert_eq!(cfg.instance_machine, elsewhere);
    for (old, new) in pads.iter().zip(&cfg.pads) {
        assert_ne!(old.id, new.id);
        assert!(is_id(&new.id));
        assert_eq!(cfg.profiles_of(&new.id).len(), migrated_count(old.kind));
    }
    assert!(cfg.profiles.iter().all(|p| cfg.pad(&p.pad_id).is_some()));
}

#[test]
fn a_renewed_pad_takes_its_profiles_along() {
    let mut cfg = migrated();
    let old = cfg.pads[1].id.clone();
    let new = cfg.renew_pad(&old).unwrap();
    assert!(is_id(&new) && new != old);
    assert_eq!(cfg.profiles[1].pad_id, new);
    assert_eq!(cfg.small_pad_index(), Some(1));
    assert!(cfg.renew_pad("0000000000000000").is_none());
}

#[test]
fn a_dashboard_pad_is_added_edited_and_removed() {
    let mut cfg = Config::default();
    let wall = cfg.add_dashboard_pad("Wall", 2, 4);
    assert_eq!(cfg.shown_pad_id(), wall);
    assert_eq!((cfg.profile().pad, cfg.profile().rows, cfg.profile().cols), (PadKind::Dashboard, 2, 4));
    assert_eq!(cfg.profile().pages[0].keys.len(), 8);
    // A new profile in the window is for that pad, with its grid.
    cfg.add_profile();
    assert_eq!(cfg.profiles_of(&wall).len(), 2);
    assert_eq!((cfg.profile().pad, cfg.profile().rows, cfg.profile().cols), (PadKind::Dashboard, 2, 4));
    cfg.set_dashboard_grid(&wall, 3, 6);
    assert!(cfg.profiles_of(&wall).iter().all(|i| cfg.profiles[*i].key_count() == 18));
    // Another virtual pad keeps its own profiles.
    let kitchen = cfg.add_dashboard_pad("Wall", 1, 3);
    assert_eq!(cfg.pad(&kitchen).unwrap().name, "Wall 2");
    assert_eq!(cfg.profile_along_pad(&kitchen, 1), cfg.pad_profile_of(&kitchen));
    assert!(cfg.rename_pad(&kitchen, "Wall").is_err());
    cfg.rename_pad(&kitchen, "Kitchen").unwrap();
    // The 293S stays; removing the virtual pad takes its profiles and goes back to the 293S.
    assert!(!cfg.remove_pad(&cfg.pads[0].id.clone()));
    cfg.show_pad_id(&wall);
    assert!(cfg.remove_pad(&wall));
    assert!(cfg.pad(&wall).is_none() && cfg.profiles_of(&wall).is_empty());
    assert_eq!(cfg.profiles.len(), 2);
    assert_eq!(cfg.profile().pad, PadKind::Main);
    cfg.repair();
    assert_eq!(cfg.pads.len(), 2);
}

#[test]
fn a_dashboard_pad_never_shows_mini_monitor() {
    let mut cfg = Config::default();
    cfg.add_dashboard_pad("Wall", 2, 4);
    cfg.mini_monitor = true;
    cfg.monitor_pad = PadKind::Dashboard;
    assert!(!cfg.monitor_on(PadKind::Dashboard));
    cfg.repair();
    assert_eq!(cfg.monitor_pad, PadKind::Main);
}

#[test]
fn each_pad_runs_its_own_profile_including_dashboards() {
    let mut cfg = both_pads();
    let wall = cfg.add_dashboard_pad("Wall", 2, 4);
    cfg.add_profile();
    let (first, second) = (cfg.profiles_of(&wall)[0], cfg.profiles_of(&wall)[1]);
    cfg.choose_profile(1);
    assert_eq!(cfg.pad_profile_of(&wall), Some(second));
    cfg.run_profile(first);
    assert_eq!(cfg.active_profile, 1, "the window stays on the 293S");
    assert_eq!(cfg.pad_profile_of(&wall), Some(first));
    assert_eq!(cfg.main_index(), 1);
}

#[test]
fn a_test_copy_keeps_its_settings_in_its_own_folder() {
    let folder = PathBuf::from("sandbox-folder");
    let own = PathBuf::from("own.json");
    assert_eq!(chosen_settings_file(Some(folder.clone()), Some(own.clone())), Some(folder.join("config.json")));
    assert_eq!(chosen_settings_file(None, Some(own.clone())), Some(own));
    assert_eq!(chosen_settings_file(None, None), None);
}

#[test]
fn a_virtual_pad_keeps_each_key_in_its_row_and_column_when_resized() {
    let mut cfg = Config::default();
    let wall = cfg.add_dashboard_pad("Wall", 2, 3);
    let titled = |title: &str| DockKey { title: title.into(), ..DockKey::default() };
    // Row 1: A B C, row 2: D E F.
    cfg.profile_mut().pages[0].keys = ["A", "B", "C", "D", "E", "F"].iter().map(|t| titled(t)).collect();
    // Two across would leave C and F out; nothing is lost going wider.
    assert_eq!(cfg.keys_outside_grid(&wall, 2, 2), 2);
    assert_eq!(cfg.keys_outside_grid(&wall, 3, 4), 0);
    cfg.set_dashboard_grid(&wall, 3, 4);
    let titles = |cfg: &Config| cfg.profile().pages[0].keys.iter().map(|k| k.title.clone()).collect::<Vec<_>>();
    assert_eq!(titles(&cfg), ["A", "B", "C", "", "D", "E", "F", "", "", "", "", ""]);
    cfg.set_dashboard_grid(&wall, 1, 2);
    assert_eq!(titles(&cfg), ["A", "B"]);
    assert_eq!(cfg.keys_outside_grid(&wall, 1, 2), 0);
}

#[test]
fn a_pad_still_called_dashboard_pad_becomes_a_virtual_pad() {
    let mut cfg = Config::default();
    let first = cfg.add_dashboard_pad("Dashboard pad", 2, 4);
    let second = cfg.add_dashboard_pad("Dashboard pad", 2, 4);
    let own = cfg.add_dashboard_pad("Dashboard pad for the hall", 1, 3);
    assert_eq!(cfg.pad(&second).unwrap().name, "Dashboard pad 2");
    cfg.repair();
    assert_eq!(cfg.pad(&first).unwrap().name, "Virtual pad");
    assert_eq!(cfg.pad(&second).unwrap().name, "Virtual pad 2");
    assert_eq!(cfg.pad(&own).unwrap().name, "Dashboard pad for the hall");
    assert_eq!(cfg.profiles[cfg.pad_profile_of(&first).unwrap()].name, "Virtual pad");
}
