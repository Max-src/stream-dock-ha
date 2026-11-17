use super::*;
use crate::ha::Entity;
use crate::model::{Action, Config, HaAction};

fn look(text: &str, style: KeyStyle) -> Look {
    Look {
        text: text.into(),
        glyph: "🌐".into(),
        color: [46, 104, 214],
        image: None,
        image_changed: None,
        fit: PictureFit::Fit,
        style,
        toggle: None,
        track: None,
        camera: None,
        graph: None,
        unavailable: false,
    }
}

fn lit_pixels_outside(canvas: &RgbImage, style: &KeyStyle, color_of_text: [u8; 3]) -> usize {
    let size = canvas.width() as f32;
    canvas
        .enumerate_pixels()
        .filter(|(x, y, px)| {
            let inset = edge_inset(style, *y as f32 + 0.5, size);
            let x = *x as f32 + 0.5;
            (x < inset - 1.0 || x > size - inset + 1.0) && px.0 == color_of_text
        })
        .count()
}

#[test]
fn text_stays_inside_every_shape_at_every_size() {
    for shape in [KeyShape::Square, KeyShape::Rounded, KeyShape::Circle] {
        for size in [9u8, 13, 18, 24] {
            for place in [TextPlace::Bottom, TextPlace::Middle, TextPlace::Top] {
                let style = KeyStyle { shape, size, place, corner: 22, ..KeyStyle::default() };
                for text in ["Volume down", "A rather long title for one key", "\nAC Fan\n28.63°C"] {
                    let picture = key_picture(&look(text, style.clone()));
                    assert_eq!(
                        lit_pixels_outside(&picture, &style, style.color),
                        0,
                        "{shape:?} size {size} {place:?}: {text}"
                    );
                }
            }
        }
    }
}

#[test]
fn corners_are_cut_for_rounded_keys_and_circles() {
    for shape in [KeyShape::Rounded, KeyShape::Circle] {
        let style = KeyStyle { shape, ..KeyStyle::default() };
        let picture = key_picture(&look("", style));
        assert_eq!(picture.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(picture.get_pixel(84, 84).0, [0, 0, 0]);
        assert_eq!(picture.get_pixel(5, 42).0, [46, 104, 214]);
    }
    let picture = key_picture(&look("", KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() }));
    assert_eq!(picture.get_pixel(0, 0).0, [46, 104, 214]);
}

#[test]
fn long_text_gets_smaller_rather_than_cut() {
    let style = KeyStyle { size: 24, ..KeyStyle::default() };
    let block = lay_out_text(&look("Bedroom temperature", style), 85.0);
    assert!(block.pen.size < 24.0);
    assert_eq!(block.lines.join(" "), "Bedroom temperature");
}

fn entities(state: &str) -> Entities {
    let entity = Entity { state: state.into(), name: "Auto AC".into(), ..Entity::default() };
    [("input_boolean.auto_ac_on".to_string(), entity)].into_iter().collect()
}

fn bound_key() -> DockKey {
    DockKey {
        actions: vec![Action::HomeAssistant(HaAction {
            entity: "input_boolean.auto_ac_on".into(),
            service: "toggle".into(),
            on_title: "{{friendly_name}} is on".into(),
            off_image: Some("off.png".into()),
            ..HaAction::default()
        })],
        ..DockKey::default()
    }
}

#[test]
fn a_home_assistant_key_follows_the_entitys_state() {
    let style = KeyStyle::default();
    let on = Look::of(&bound_key(), &style, &entities("on")).unwrap();
    assert_eq!(on.text, "Auto AC is on");
    assert_eq!(on.toggle, Some(true));
    let off = Look::of(&bound_key(), &style, &entities("off")).unwrap();
    assert_eq!(off.text, "Auto AC");
    assert_eq!(off.image.as_deref(), Some("off.png"));
    assert_eq!(off.toggle, None);
    let unknown = Look::of(&bound_key(), &style, &Entities::new()).unwrap();
    assert_eq!(unknown.text, "input_boolean.auto_ac_on");
    assert!(on != off);
}

fn colored_key() -> DockKey {
    let mut key = bound_key();
    key.color = [10, 20, 30];
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.off_image = None;
        binding.on_color = Some([32, 138, 104]);
        binding.off_color = Some([186, 58, 78]);
    }
    key
}

#[test]
fn the_background_can_follow_the_entitys_state() {
    let style = KeyStyle::default();
    let color_for = |state: &str| Look::of(&colored_key(), &style, &entities(state)).unwrap().color;
    assert_eq!(color_for("on"), [32, 138, 104]);
    assert_eq!(color_for("off"), [186, 58, 78]);
    assert_eq!(color_for("unavailable"), [10, 20, 30]);
    let unknown = Look::of(&colored_key(), &style, &Entities::new()).unwrap();
    assert_eq!(unknown.color, [10, 20, 30]);
    let plain = Look::of(&bound_key(), &style, &entities("on")).unwrap();
    assert_eq!(plain.color, DockKey::default().color);
}

/// How many pixels of a key are exactly this colour.
fn pixels_of(picture: &RgbImage, color: [u8; 3]) -> usize {
    picture.pixels().filter(|p| p.0 == color).count()
}

#[test]
fn the_switch_track_can_have_its_own_colour_for_each_state() {
    let style = KeyStyle::default();
    let (red, blue) = ([200, 0, 0], [0, 0, 200]);
    let mut key = colored_key();
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.on_color = None;
        binding.off_color = None;
        binding.on_track = Some(red);
        binding.off_track = Some(blue);
    }
    let drawn = |state: &str| key_picture(&Look::of(&key, &style, &entities(state)).unwrap());
    let (on, off) = (drawn("on"), drawn("off"));
    assert!(pixels_of(&on, red) > 300 && pixels_of(&on, blue) == 0);
    assert!(pixels_of(&off, blue) > 300 && pixels_of(&off, red) == 0);

    // Without a choice it stays yellow, and a choice is honoured even on
    // a key of the same colour.
    let plain = key_picture(&Look::of(&bound_key_with_switch(), &style, &entities("on")).unwrap());
    assert!(pixels_of(&plain, SWITCH_ON) > 300);
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.on_track = Some(key.color);
    }
    let same = Look::of(&key, &style, &entities("on")).unwrap();
    assert_eq!(same.track, Some(key.color));
}

fn bound_key_with_switch() -> DockKey {
    let mut key = bound_key();
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.on_title.clear();
    }
    key
}

#[test]
fn a_black_key_stays_black_with_its_switch_and_text_showing() {
    let style = KeyStyle::default();
    let mut key = bound_key_with_switch();
    key.color = [0, 0, 0];
    let picture = key_picture(&Look::of(&key, &style, &entities("on")).unwrap());
    assert_eq!(picture.get_pixel(1, 1).0, [0, 0, 0]);
    assert!(pixels_of(&picture, SWITCH_ON) > 300);
    let text = picture.enumerate_pixels().filter(|(_, y, p)| *y > 55 && brightness(p.0) > 120.0).count();
    assert!(text > 20, "only {text} bright pixels of text");
}

#[test]
fn keys_stay_readable_on_any_background() {
    for background in [[255, 255, 255], [240, 180, 20], [92, 98, 112], [0, 0, 0], [186, 58, 78]] {
        for wanted in [[255, 255, 255], [240, 180, 20], [92, 98, 112], [24, 26, 32]] {
            let shown = visible_on(background, wanted, SHAPE_CONTRAST);
            assert!((brightness(shown) - brightness(background)).abs() >= SHAPE_CONTRAST, "{wanted:?} on {background:?}");
        }
    }
    // The usual grey switch stays grey on an ordinary dark key.
    assert_eq!(visible_on(DockKey::default().color, [92, 98, 112], SHAPE_CONTRAST), [92, 98, 112]);
}

#[test]
fn sensor_icons_differ_from_each_other() {
    let style = KeyStyle::default();
    let tile = |icon| {
        let face = Face { icon: Some(icon), value: "37%".into(), percent: Some(37), ..Face::default() };
        band_picture(&face, &style).into_raw()
    };
    let (cpu, gpu, memory) = (tile(BandIcon::Cpu), tile(BandIcon::Gpu), tile(BandIcon::Memory));
    let differing = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    assert!(differing(&cpu, &gpu) > 600);
    assert!(differing(&gpu, &memory) > 600);
    assert!(differing(&cpu, &memory) > 600);
}

fn temperature_key() -> DockKey {
    DockKey {
        color: [10, 20, 30],
        actions: vec![Action::HomeAssistant(HaAction {
            entity: "sensor.made_up_room".into(),
            level_colors: vec![
                crate::model::MeterLevel { from: 18.0, color: [0, 0, 200] },
                crate::model::MeterLevel { from: 26.0, color: [230, 0, 0] },
            ],
            state_colors: vec![crate::model::StateColor { state: "unavailable".into(), color: [90, 90, 90], text: None }],
            ..HaAction::default()
        })],
        ..DockKey::default()
    }
}

#[test]
fn a_keys_background_follows_a_temperature_and_its_states() {
    let style = KeyStyle::default();
    let color_at = |state: &str| {
        let entity = Entity { state: state.into(), ..Entity::default() };
        let found: Entities = [("sensor.made_up_room".to_string(), entity)].into_iter().collect();
        Look::of(&temperature_key(), &style, &found).unwrap().color
    };
    assert_eq!(color_at("12"), [10, 20, 30]);
    assert_eq!(color_at("20.5"), [0, 0, 200]);
    assert_eq!(color_at("27"), [230, 0, 0]);
    assert_eq!(color_at("unavailable"), [90, 90, 90]);
    assert_eq!(color_at("unknown"), [10, 20, 30]);
}

#[test]
fn a_keys_state_colour_comes_before_its_on_and_off_colours() {
    let style = KeyStyle::default();
    let mut key = colored_key();
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.state_colors = vec![crate::model::StateColor { state: "on".into(), color: [1, 2, 3], text: None }];
    }
    assert_eq!(Look::of(&key, &style, &entities("on")).unwrap().color, [1, 2, 3]);
    assert_eq!(Look::of(&key, &style, &entities("off")).unwrap().color, [186, 58, 78]);
}

#[test]
fn a_keys_text_can_have_a_colour_for_each_state() {
    let style = KeyStyle::default();
    let mut key = colored_key();
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.state_colors = vec![crate::model::StateColor { state: "on".into(), color: [1, 2, 3], text: Some([200, 0, 0]) }];
    }
    let on = Look::of(&key, &style, &entities("on")).unwrap();
    assert_eq!(on.style.color, [200, 0, 0]);
    // Small letters blend with their outline, so few pixels are exactly this colour.
    let reddish = key_picture(&on).pixels().filter(|p| p.0[0] > 110 && p.0[0] > p.0[1].saturating_add(60)).count();
    assert!(reddish > 5, "only {reddish} reddish pixels");
    assert_eq!(Look::of(&key, &style, &entities("off")).unwrap().style.color, style.color);
}

#[test]
fn a_band_value_takes_its_states_colour_with_every_meter() {
    let green = [0, 200, 0];
    for meter in MeterStyle::ALL {
        let face = Face {
            value: "heating".into(),
            value_color: Some(green),
            percent: Some(40),
            meter,
            ..Face::default()
        };
        assert!(pixels_of(&band_picture(&face, &KeyStyle::default()), green) > 10, "{meter:?}");
    }
    let face = Face { value: "heating".into(), value_color: Some(green), ..Face::default() };
    assert!(pixels_of(&band_picture(&face, &KeyStyle::default()), green) > 10);
}

fn meter_tile(meter: MeterStyle, percent: u8) -> RgbImage {
    let (green, orange, red) = ([0, 200, 0], [250, 150, 0], [230, 0, 0]);
    let face = Face {
        label: "CPU".into(),
        value: format!("{percent}%"),
        percent: Some(percent),
        meter,
        levels: vec![(0, green), (60, orange), (85, red)],
        history: vec![20, 70, 90],
        ..Face::default()
    };
    band_picture(&face, &KeyStyle::default())
}

#[test]
fn a_reading_takes_the_colour_of_the_level_it_has_reached() {
    let levels = [(0, [1, 1, 1]), (60, [2, 2, 2]), (85, [3, 3, 3])];
    assert_eq!(level_color(&levels, 0.0), [1, 1, 1]);
    assert_eq!(level_color(&levels, 59.0), [1, 1, 1]);
    assert_eq!(level_color(&levels, 60.0), [2, 2, 2]);
    assert_eq!(level_color(&levels, 100.0), [3, 3, 3]);
    assert_eq!(level_color(&[], 50.0), METER_PLAIN);
}

#[test]
fn a_bar_is_one_colour_and_changes_with_the_level() {
    let (green, orange, red) = ([0, 200, 0], [250, 150, 0], [230, 0, 0]);
    let at = |percent| {
        let tile = meter_tile(MeterStyle::Bar, percent);
        (pixels_of(&tile, green) > 0, pixels_of(&tile, orange) > 0, pixels_of(&tile, red) > 0)
    };
    assert_eq!(at(40), (true, false, false));
    assert_eq!(at(70), (false, true, false));
    assert_eq!(at(95), (false, false, true));
}

#[test]
fn a_ring_is_one_colour_and_fills_with_the_reading() {
    let orange = [250, 150, 0];
    let less = pixels_of(&meter_tile(MeterStyle::Ring, 61), orange);
    let more = pixels_of(&meter_tile(MeterStyle::Ring, 80), orange);
    assert!(more > less && less > 0);
    assert_eq!(pixels_of(&meter_tile(MeterStyle::Ring, 61), [230, 0, 0]), 0);
}

#[test]
fn blocks_and_gauges_show_every_level_passed() {
    let (green, orange, red) = ([0, 200, 0], [250, 150, 0], [230, 0, 0]);
    for meter in [MeterStyle::Blocks, MeterStyle::Gauge] {
        let high = meter_tile(meter, 95);
        assert!(pixels_of(&high, green) > 0 && pixels_of(&high, orange) > 0 && pixels_of(&high, red) > 0, "{meter:?}");
        let low = meter_tile(meter, 30);
        assert!(pixels_of(&low, green) > 0 && pixels_of(&low, orange) == 0 && pixels_of(&low, red) == 0, "{meter:?}");
    }
}

#[test]
fn a_full_row_of_blocks_stays_on_the_tile() {
    let tile = meter_tile(MeterStyle::Blocks, 100);
    assert_eq!(*tile.get_pixel(73, 65), Rgb([230, 0, 0]));
    assert_eq!(*tile.get_pixel(5, 65), Rgb(BAND_BACKGROUND));
    assert_eq!(*tile.get_pixel(74, 65), Rgb(BAND_BACKGROUND));
}

#[test]
fn a_gauge_needle_follows_the_reading() {
    let needle = KeyStyle::default().color;
    let left_of_center = |tile: &RgbImage| (10..36).map(|x| (10..48).filter(|y| tile.get_pixel(x, *y).0 == needle).count()).sum::<usize>();
    let right_of_center = |tile: &RgbImage| (44..70).map(|x| (18..48).filter(|y| tile.get_pixel(x, *y).0 == needle).count()).sum::<usize>();
    let (low, high) = (meter_tile(MeterStyle::Gauge, 10), meter_tile(MeterStyle::Gauge, 90));
    assert!(left_of_center(&low) > 5 && right_of_center(&low) == 0);
    assert!(right_of_center(&high) > 5);
}

#[test]
fn a_graph_draws_each_reading_in_its_own_colour_with_the_newest_on_the_right() {
    let tile = meter_tile(MeterStyle::Graph, 90);
    let top_of = |x: u32| (25..72).find(|y| tile.get_pixel(x, *y).0 != [30, 33, 41]);
    assert_eq!(*tile.get_pixel(71, top_of(71).unwrap()), Rgb([230, 0, 0]));
    assert_eq!(*tile.get_pixel(70, top_of(70).unwrap()), Rgb([250, 150, 0]));
    assert_eq!(*tile.get_pixel(69, top_of(69).unwrap()), Rgb([0, 200, 0]));
    assert!(top_of(71).unwrap() < top_of(69).unwrap());
    assert_eq!(top_of(68), None);
}

#[test]
fn a_number_takes_the_colour_of_its_level() {
    let tile = meter_tile(MeterStyle::Number, 70);
    assert!(pixels_of(&tile, [250, 150, 0]) > 30);
}

#[test]
fn a_full_history_fills_the_graph_exactly() {
    let face = Face {
        value: "50%".into(),
        percent: Some(50),
        meter: MeterStyle::Graph,
        levels: vec![(0, [0, 200, 0])],
        history: vec![50; 200],
        ..Face::default()
    };
    let tile = band_picture(&face, &KeyStyle::default());
    assert_eq!(*tile.get_pixel(8, 71), Rgb(faded([0, 200, 0])));
    assert_eq!(*tile.get_pixel(71, 71), Rgb(faded([0, 200, 0])));
    assert_eq!(*tile.get_pixel(7, 71), Rgb(BAND_BACKGROUND));
}

#[test]
fn a_value_too_wide_for_a_graph_leaves_out_the_name() {
    let face = Face {
        label: "Energy".into(),
        value: "12345678.901 kWh".into(),
        percent: Some(40),
        meter: MeterStyle::Graph,
        ..Face::default()
    };
    let style = KeyStyle { bold: true, ..KeyStyle::default() };
    assert_eq!(band_picture(&face, &style).width(), BAND_SIZE);
}

#[test]
fn every_meter_style_looks_different() {
    let tiles: Vec<Vec<u8>> = MeterStyle::ALL.iter().map(|meter| meter_tile(*meter, 70).into_raw()).collect();
    for (first, a) in tiles.iter().enumerate() {
        for b in &tiles[first + 1..] {
            assert!(a.iter().zip(b).filter(|(x, y)| x != y).count() > 300);
        }
    }
}

#[test]
fn long_names_and_values_stay_on_the_tile_with_every_meter() {
    for meter in MeterStyle::ALL {
        let face = Face {
            label: "Living room".into(),
            value: "1234.56 kWh".into(),
            percent: Some(40),
            meter,
            history: vec![40; 64],
            ..Face::default()
        };
        let tile = band_picture(&face, &KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() });
        for y in 0..80 {
            assert_eq!(*tile.get_pixel(0, y), Rgb(BAND_BACKGROUND), "{meter:?}");
            assert_eq!(*tile.get_pixel(79, y), Rgb(BAND_BACKGROUND), "{meter:?}");
        }
    }
}

fn sensor_key(id: &str) -> DockKey {
    DockKey {
        actions: vec![Action::HomeAssistant(HaAction { entity: id.into(), ..HaAction::default() })],
        ..DockKey::default()
    }
}

fn sensor_at_home(id: &str, state: &str, class: &str) -> Entities {
    let entity = Entity { state: state.into(), device_class: class.into(), ..Entity::default() };
    [(id.to_string(), entity)].into_iter().collect()
}

/// Light pixels above the text, which is where an icon goes.
fn icon_pixels(key: &DockKey, entities: &Entities) -> usize {
    let picture = key_picture(&Look::of(key, &KeyStyle::default(), entities).unwrap());
    picture.enumerate_pixels().filter(|(_, y, p)| *y < 45 && brightness(p.0) > 150.0).count()
}

#[test]
fn a_temperature_sensor_shows_a_thermometer_on_its_key() {
    let home = sensor_at_home("sensor.bedroom", "21.5", "temperature");
    let key = sensor_key("sensor.bedroom");
    assert_eq!(Look::of(&key, &KeyStyle::default(), &home).unwrap().glyph, "mdi:thermometer");
    assert!(icon_pixels(&key, &home) > 100);
}

#[test]
fn a_door_sensor_shows_a_door_rather_than_a_switch() {
    let home = sensor_at_home("binary_sensor.front", "on", "door");
    let look = Look::of(&sensor_key("binary_sensor.front"), &KeyStyle::default(), &home).unwrap();
    assert_eq!(look.toggle, None);
    assert_eq!(look.glyph, "mdi:door-open");
}

#[test]
fn the_symbol_missing_from_the_built_in_fonts_now_draws() {
    let key = DockKey { glyph: "🌡".into(), ..DockKey::default() };
    let lit = icon_pixels(&key, &Entities::new());
    assert!(lit > 40, "only {lit} lit pixels");
}

#[test]
fn any_key_can_have_no_icon_its_own_icon_or_a_picture() {
    let style = KeyStyle::default();
    let home = entities("on");
    let mut key = bound_key_with_switch();
    let look = |key: &DockKey| Look::of(key, &style, &home).unwrap();
    // Left alone: the switch.
    assert_eq!(look(&key).toggle, Some(true));
    assert_eq!(look(&key).glyph, "");
    // An icon of its own takes over from the switch.
    key.glyph = "mdi:fan".into();
    assert_eq!(look(&key).toggle, None);
    assert_eq!(look(&key).glyph, "mdi:fan");
    // A picture takes over from both.
    key.image = Some("a.png".into());
    assert_eq!(look(&key).image.as_deref(), Some("a.png"));
    assert_eq!(look(&key).glyph, "");
    // No icon really is nothing, even with a picture for this state.
    key.glyph.clear();
    key.image = None;
    key.no_icon = true;
    if let Action::HomeAssistant(binding) = &mut key.actions[0] {
        binding.on_image = Some("on.png".into());
    }
    assert_eq!(look(&key).toggle, None);
    assert_eq!(look(&key).glyph, "");
    assert_eq!(look(&key).image, None);
    assert_eq!(icon_pixels(&key, &home), 0);
}

#[test]
fn an_old_settings_file_keeps_its_keys() {
    let key: DockKey = serde_json::from_str(r#"{"title":"Old","glyph":"🌐"}"#).unwrap();
    assert!(!key.no_icon);
    assert_eq!(key.glyph, "🌐");
}

#[test]
fn a_band_tile_can_show_an_entitys_icon() {
    let style = KeyStyle::default();
    let tile = |symbol: Option<&str>| {
        let face = Face { symbol: symbol.map(str::to_string), value: "21.5°C".into(), ..Face::default() };
        band_picture(&face, &style).into_raw()
    };
    let differing = tile(Some("mdi:thermometer")).iter().zip(tile(None)).filter(|(a, b)| **a != *b).count();
    assert!(differing > 300);
}

/// A picture with a green circle in its middle, on black.
fn circle_picture(name: &str, width: u32, height: u32) -> String {
    let radius = width.min(height) as f32 * 0.45;
    let picture = RgbImage::from_fn(width, height, |x, y| {
        let distance = (x as f32 - width as f32 / 2.0).hypot(y as f32 - height as f32 / 2.0);
        Rgb(if distance < radius { [0, 255, 0] } else { [0, 0, 0] })
    });
    let path = std::env::temp_dir().join(format!("stream-dock-ha-{name}.png"));
    picture.save(&path).unwrap();
    path.display().to_string()
}

/// How wide and tall the green part of a key is.
fn green_size(picture: &RgbImage) -> (u32, u32) {
    let green: Vec<(u32, u32)> =
        picture.enumerate_pixels().filter(|(_, _, p)| p.0[1] > 128 && p.0[0] < 100).map(|(x, y, _)| (x, y)).collect();
    let span = |values: Vec<u32>| values.iter().max().unwrap() - values.iter().min().unwrap() + 1;
    (span(green.iter().map(|g| g.0).collect()), span(green.iter().map(|g| g.1).collect()))
}

#[test]
fn a_circle_stays_round_whatever_the_pictures_shape() {
    for (name, width, height) in [("wide", 900, 300), ("tall", 300, 900), ("square", 500, 500), ("small", 60, 30)] {
        let path = circle_picture(name, width, height);
        for title in ["", "A title"] {
            let mut look = look(title, KeyStyle::default());
            look.image = Some(path.clone());
            look.color = [0, 0, 0];
            let (across, down) = green_size(&key_picture(&look));
            println!("{name} {title:?}: circle {across} by {down}");
            assert!(across.abs_diff(down) <= 1, "{name}: {across} by {down}");
        }
    }
}

/// A wide picture: red, then green, then blue, a third each.
fn three_colours(name: &str) -> String {
    let picture = RgbImage::from_fn(900, 300, |x, _| {
        Rgb(match x / 300 {
            0 => [255, 0, 0],
            1 => [0, 255, 0],
            _ => [0, 0, 255],
        })
    });
    let path = std::env::temp_dir().join(format!("stream-dock-ha-{name}.png"));
    picture.save(&path).unwrap();
    path.display().to_string()
}

fn drawn(path: &str, fit: PictureFit, title: &str) -> RgbImage {
    let mut look = look(title, KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() });
    look.image = Some(path.to_string());
    look.fit = fit;
    look.color = [9, 9, 9];
    key_picture(&look)
}

#[test]
fn the_whole_picture_shows_every_part_with_bars() {
    let picture = drawn(&three_colours("fit"), PictureFit::Fit, "");
    assert_eq!(picture.get_pixel(42, 2).0, [9, 9, 9]);
    assert!(picture.get_pixel(3, 42).0[0] > 200);
    assert!(picture.get_pixel(81, 42).0[2] > 200);
}

#[test]
fn fill_covers_the_key_and_cuts_off_the_sides() {
    let picture = drawn(&three_colours("fill"), PictureFit::Fill, "");
    for (x, y) in [(1, 1), (83, 1), (1, 83), (83, 83), (42, 42)] {
        assert!(picture.get_pixel(x, y).0[1] > 200, "at {x},{y}");
    }
}

#[test]
fn stretch_covers_the_key_and_keeps_every_part() {
    let picture = drawn(&three_colours("stretch"), PictureFit::Stretch, "");
    assert!(picture.get_pixel(3, 2).0[0] > 200);
    assert!(picture.get_pixel(42, 82).0[1] > 200);
    assert!(picture.get_pixel(81, 2).0[2] > 200);
}

#[test]
fn original_size_shows_the_middle_of_a_large_picture_unchanged() {
    let large = drawn(&three_colours("original"), PictureFit::Original, "");
    assert!(large.get_pixel(2, 42).0[1] > 200 && large.get_pixel(82, 42).0[1] > 200);
    let small = circle_picture("original-small", 40, 40);
    let canvas = drawn(&small, PictureFit::Original, "");
    // Exactly the pixels a circle of radius 18 has, so nothing was resized.
    assert_eq!(green_size(&canvas), (35, 35));
    assert_eq!(canvas.get_pixel(5, 5).0, [9, 9, 9]);
}

#[test]
fn a_covering_picture_runs_under_the_title_which_has_an_outline() {
    let path = three_colours("title");
    let with_title = drawn(&path, PictureFit::Fill, "Title");
    // The picture reaches the bottom edge, with nothing dimmed behind the words.
    assert!(with_title.get_pixel(1, 72).0[1] > 200);
    let white = with_title.enumerate_pixels().filter(|(_, y, p)| *y > 60 && p.0.iter().all(|v| *v > 200)).count();
    let black = with_title.enumerate_pixels().filter(|(_, y, p)| *y > 60 && p.0.iter().all(|v| *v < 60)).count();
    assert!(white > 10 && black > 10, "{white} white and {black} black pixels");
    // The whole picture keeps clear of the title instead.
    let kept_clear = drawn(&path, PictureFit::Fit, "Title");
    assert_eq!(kept_clear.get_pixel(1, 76).0, [9, 9, 9]);
}

#[test]
fn a_title_outline_is_the_opposite_colour() {
    assert_eq!(opposite([255, 255, 255]), [0, 0, 0]);
    assert_eq!(opposite([255, 214, 90]), [0, 41, 165]);
}

#[test]
fn square_titles_sit_where_the_original_puts_them() {
    // A line 13 pixels high, on the 85 pixel key.
    assert_eq!(title_lines(TextPlace::Top, 1, 13, 10, 85), (8, 13));
    assert_eq!(title_lines(TextPlace::Bottom, 1, 13, 10, 85), (84, 13));
    assert_eq!(title_lines(TextPlace::Bottom, 2, 13, 10, 85), (71, 13));
    assert_eq!(title_lines(TextPlace::Middle, 1, 13, 10, 85), (46, 13));
    // Taller lines use the other rule for the top.
    assert_eq!(title_lines(TextPlace::Top, 1, 14, 11, 85), (9, 14));
}

#[test]
fn a_band_tile_can_fill_too() {
    let path = three_colours("band");
    let tile = |picture_fit| {
        let face = Face { picture: Some(path.clone()), picture_fit, ..Face::default() };
        band_picture(&face, &KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() })
    };
    assert!(tile(PictureFit::Fill).get_pixel(40, 2).0[1] > 200);
    assert_eq!(tile(PictureFit::Fit).get_pixel(40, 2).0, BAND_BACKGROUND);
}

#[test]
fn saved_keys_and_tiles_from_before_keep_the_whole_picture() {
    let key: DockKey = serde_json::from_str(r#"{"title":"Old","image":"a.png"}"#).unwrap();
    assert_eq!(key.picture_fit, PictureFit::Fit);
    let tile: crate::model::BandItem = serde_json::from_str(r#"{"kind":"picture","path":"a.png"}"#).unwrap();
    assert_eq!(tile, crate::model::BandItem::Picture { path: "a.png".into(), fit: PictureFit::Fit });
}

#[test]
fn windows_fonts_are_listed_and_usable() {
    let fonts = installed_fonts();
    assert!(fonts.iter().any(|(name, _)| name.starts_with("Arial") || name.starts_with("Segoe")));
    let (_, file) = fonts.iter().find(|(name, _)| name.starts_with("Segoe UI") || name.starts_with("Arial")).unwrap();
    assert!(system_font(file).is_some());
    assert!(system_font("no-such-font.ttf").is_none());
}

/// Contact sheet of shapes and styles, to check by eye:
/// `PAD_PREVIEW=file.png cargo test sheet`
#[test]
fn sheet() {
    let Some(path) = std::env::var_os("PAD_PREVIEW") else { return };
    let cfg = Config::sample();
    let keys = &cfg.profiles[0].pages[0].keys;
    let styles = [
        KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() },
        KeyStyle::default(),
        KeyStyle { shape: KeyShape::Circle, ..KeyStyle::default() },
        KeyStyle { shape: KeyShape::Rounded, corner: 26, size: 11, bold: true, color: [255, 214, 90], ..KeyStyle::default() },
        KeyStyle { font: "georgia.ttf".into(), italic: true, place: TextPlace::Top, ..KeyStyle::default() },
    ];
    let mut sheet = RgbImage::from_pixel(5 * 95 + 10, 5 * 95 + 10, Rgb([10, 10, 12]));
    for (row, style) in styles.iter().enumerate() {
        for (column, index) in [0usize, 8, 9, 2].into_iter().enumerate() {
            let picture = key_picture(&Look::of(&keys[index], style, &Entities::new()).unwrap());
            image::imageops::overlay(&mut sheet, &picture, 10 + column as i64 * 95, 10 + row as i64 * 95);
        }
        let mut ha = bound_key();
        if let Action::HomeAssistant(binding) = &mut ha.actions[0] {
            binding.off_image = None;
        }
        ha.color = [0, 0, 0];
        let picture = key_picture(&Look::of(&ha, style, &entities("on")).unwrap());
        image::imageops::overlay(&mut sheet, &picture, 10 + 4 * 95, 10 + row as i64 * 95);
    }
    sheet.save(path).unwrap();
}

/// A wide range of keys and band tiles: every shape, text place and meter,
/// switches, pictures with each fit, icons, and a camera alone and stretched.
/// Drawn at `px` pixels, by name.
fn gallery(px: u32) -> Vec<(String, RgbImage)> {
    let mut pictures = Vec::new();
    let cfg = Config::sample();
    let keys = &cfg.profiles[0].pages[0].keys;
    let styles = [
        KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() },
        KeyStyle::default(),
        KeyStyle { shape: KeyShape::Circle, ..KeyStyle::default() },
        KeyStyle { shape: KeyShape::Rounded, corner: 26, size: 11, bold: true, color: [255, 214, 90], ..KeyStyle::default() },
        KeyStyle { font: "georgia.ttf".into(), italic: true, place: TextPlace::Top, ..KeyStyle::default() },
        KeyStyle { shape: KeyShape::Square, place: TextPlace::Middle, size: 24, ..KeyStyle::default() },
    ];
    for (s, style) in styles.iter().enumerate() {
        for (k, key) in keys.iter().enumerate() {
            if let Some(look) = Look::of(key, style, &Entities::new()) {
                pictures.push((format!("sample {s} {k}"), key_picture_sized(&look, px)));
            }
        }
        for state in ["on", "off"] {
            let mut ha = bound_key();
            if let Action::HomeAssistant(binding) = &mut ha.actions[0] {
                binding.off_image = None;
                binding.show_switch = true;
                binding.on_track = Some([20, 200, 90]);
            }
            let look = Look::of(&ha, style, &entities(state)).unwrap();
            pictures.push((format!("switch {s} {state}"), key_picture_sized(&look, px)));
        }
        pictures.push((format!("long {s}"), key_picture_sized(&look("A rather long title for one key", style.clone()), px)));
        pictures.push((format!("empty {s}"), empty_picture_sized(style, px)));
    }
    // A file for each test thread, as several tests draw the gallery at once.
    let thread = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "-");
    let circle = circle_picture(&format!("gallery-{thread}"), 300, 200);
    for fit in [PictureFit::Fit, PictureFit::Fill, PictureFit::Stretch, PictureFit::Original] {
        for title in ["", "Garden"] {
            let mut look = look(title, KeyStyle::default());
            look.image = Some(circle.clone());
            look.fit = fit;
            pictures.push((format!("picture {fit:?} {title}"), key_picture_sized(&look, px)));
        }
    }
    let camera = "camera.render_gallery";
    crate::camera::store(camera, RgbImage::from_fn(320, 180, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])));
    for (at, span) in [([0, 0], [1, 1]), ([1, 0], [2, 2]), ([0, 1], [2, 2])] {
        let mut look = look("", KeyStyle::default());
        look.camera = Some(CameraPart {
            entity: camera.into(),
            version: crate::camera::version(camera),
            at,
            span,
            fit: PictureFit::Fill,
            gaps: [52, 56],
            shape: KeyShape::Rounded,
            corner: 14,
            outside: [40, 44, 52],
        });
        pictures.push((format!("camera {at:?} {span:?}"), key_picture_sized(&look, px)));
    }
    let (green, orange, red) = ([0, 200, 0], [250, 150, 0], [230, 0, 0]);
    let history: Vec<u8> = (0..64).map(|i| ((i * 37) % 100) as u8).collect();
    for meter in MeterStyle::ALL {
        for (heading, icon, symbol) in [("CPU", None, None), ("", Some(BandIcon::Gpu), None), ("", None, Some("mdi:thermometer"))] {
            let face = Face {
                label: heading.into(),
                value: "72%".into(),
                percent: Some(72),
                meter,
                icon,
                symbol: symbol.map(str::to_string),
                levels: vec![(0, green), (60, orange), (85, red)],
                history: history.clone(),
                ..Face::default()
            };
            for (s, style) in styles.iter().enumerate().take(3) {
                pictures.push((format!("band {meter:?} {heading} {icon:?} {} {s}", symbol.unwrap_or("-")), band_picture_sized(&face, style, px * BAND_SIZE / KEY_SIZE)));
            }
        }
    }
    for icon in [BandIcon::Cpu, BandIcon::Memory] {
        let face = Face { icon: Some(icon), value: "41%".into(), ..Face::default() };
        pictures.push((format!("band icon {icon:?}"), band_picture_sized(&face, &KeyStyle::default(), px * BAND_SIZE / KEY_SIZE)));
    }
    let face = Face { label: "Living room".into(), value: "21.5°C".into(), value_color: Some(orange), ..Face::default() };
    pictures.push(("band text".into(), band_picture_sized(&face, &KeyStyle::default(), px * BAND_SIZE / KEY_SIZE)));
    let face = Face { picture: Some(circle.clone()), picture_fit: PictureFit::Fill, ..Face::default() };
    pictures.push(("band picture".into(), band_picture_sized(&face, &styles[2], px * BAND_SIZE / KEY_SIZE)));
    pictures
}

/// A fingerprint of a picture's pixels that doesn't change between Rust versions.
fn fingerprint(picture: &RgbImage) -> u64 {
    picture.as_raw().iter().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x100000001b3))
}

/// Prints the fingerprints of the gallery at the usual sizes:
/// `cargo test print_gallery -- --ignored --nocapture`
#[test]
#[ignore = "only to record the fingerprints"]
fn print_gallery() {
    for (name, picture) in gallery(KEY_SIZE) {
        println!("    (\"{name}\", 0x{:016x}),", fingerprint(&picture));
    }
}

/// The gallery's fingerprints at 85 and 80 pixels, recorded before pictures
/// could be drawn at other sizes. They depend on the fonts Windows has, so on
/// another PC they may need recording again with `print_gallery`.
const GALLERY_AT_USUAL_SIZE: &[(&str, u64)] = &[
    ("sample 0 0", 0x3a3447592e97dbbc),
    ("sample 0 1", 0x1861fb54851a1ad5),
    ("sample 0 2", 0xd48d6f6253e7fedc),
    ("sample 0 3", 0x0a893bbb02f5ccac),
    ("sample 0 4", 0xa2284caa96863894),
    ("sample 0 5", 0xed090bb87a2a6d80),
    ("sample 0 6", 0x8c7e8a4afbd5b3d2),
    ("sample 0 7", 0x8e74bc3f9998869a),
    ("sample 0 8", 0x706ab92594fa20ff),
    ("sample 0 9", 0xfef52625ac70a946),
    ("sample 0 10", 0xbc0b868c99a9adec),
    ("sample 0 11", 0x8d5703d73412b641),
    ("sample 0 12", 0x97a6b9af0b6ff5d1),
    ("sample 0 13", 0x58eab8ab888f4894),
    ("switch 0 on", 0x4097212453038af5),
    ("switch 0 off", 0x892be26d09402470),
    ("long 0", 0x9b7a855076622ea0),
    ("empty 0", 0xd541f758263ceebb),
    ("sample 1 0", 0x3a3447592e97dbbc),
    ("sample 1 1", 0x1861fb54851a1ad5),
    ("sample 1 2", 0xd48d6f6253e7fedc),
    ("sample 1 3", 0x0a893bbb02f5ccac),
    ("sample 1 4", 0xa2284caa96863894),
    ("sample 1 5", 0xed090bb87a2a6d80),
    ("sample 1 6", 0x8c7e8a4afbd5b3d2),
    ("sample 1 7", 0x8e74bc3f9998869a),
    ("sample 1 8", 0x706ab92594fa20ff),
    ("sample 1 9", 0xfef52625ac70a946),
    ("sample 1 10", 0xbc0b868c99a9adec),
    ("sample 1 11", 0x8d5703d73412b641),
    ("sample 1 12", 0x97a6b9af0b6ff5d1),
    ("sample 1 13", 0x58eab8ab888f4894),
    ("switch 1 on", 0x4097212453038af5),
    ("switch 1 off", 0x892be26d09402470),
    ("long 1", 0x9b7a855076622ea0),
    ("empty 1", 0xd541f758263ceebb),
    ("sample 2 0", 0x08d4e059ebad2728),
    ("sample 2 1", 0xfcde038be5591404),
    ("sample 2 2", 0x4d3388bd1841562b),
    ("sample 2 3", 0x29d09e670439f101),
    ("sample 2 4", 0x3b26f3f4cfdeb424),
    ("sample 2 5", 0x8b00450d0c07160d),
    ("sample 2 6", 0xfc437e7cb25a7ee2),
    ("sample 2 7", 0xfdc4323003310188),
    ("sample 2 8", 0x89c20f9ca07ee042),
    ("sample 2 9", 0x31f4bbb17f409733),
    ("sample 2 10", 0xa084d457d0fbe140),
    ("sample 2 11", 0x1d9c5179069d9e22),
    ("sample 2 12", 0x08cfc1c06e210852),
    ("sample 2 13", 0x59463ff5d09e9805),
    ("switch 2 on", 0xd50c96bd4b2eecf7),
    ("switch 2 off", 0x2cfddb805f94f30f),
    ("long 2", 0xcec46b528893d8ec),
    ("empty 2", 0x2384dc3bf90910ab),
    ("sample 3 0", 0x351e9095da28d3d2),
    ("sample 3 1", 0x90d26156f7f47517),
    ("sample 3 2", 0xcced6c4fdbaf301b),
    ("sample 3 3", 0xe152dd641b169987),
    ("sample 3 4", 0x0b7a1d8c060fe3d6),
    ("sample 3 5", 0xa9d0668f75c5ec59),
    ("sample 3 6", 0xffa82e2873fc24fe),
    ("sample 3 7", 0xdd7a36e97eb1d512),
    ("sample 3 8", 0xe8c01d2a9e68f146),
    ("sample 3 9", 0x57c6cd492bf55692),
    ("sample 3 10", 0xae4d57965b834da1),
    ("sample 3 11", 0x78ec6851cad78de4),
    ("sample 3 12", 0x8638f9841bfbff0b),
    ("sample 3 13", 0xf612f0bbff7f26bd),
    ("switch 3 on", 0xb92614f7a81a0032),
    ("switch 3 off", 0xbd9c391cad715034),
    ("long 3", 0x4227a12cc173f03a),
    ("empty 3", 0x4448d13ce3c5c2bb),
    ("sample 4 0", 0x42697b374cf4d5ea),
    ("sample 4 1", 0xf0bed5a6dc3949d5),
    ("sample 4 2", 0x0de2a6bdb49822fb),
    ("sample 4 3", 0x3321b38392930073),
    ("sample 4 4", 0xa630159e38f7b6f3),
    ("sample 4 5", 0x0924572fd96859f2),
    ("sample 4 6", 0x700640f8365458ed),
    ("sample 4 7", 0x30e5a7cf42137d00),
    ("sample 4 8", 0x829755fe1e8e8680),
    ("sample 4 9", 0x4d9fd154fd579d3b),
    ("sample 4 10", 0x5568378cc0ea3c01),
    ("sample 4 11", 0x4ef93f8d6dec33c8),
    ("sample 4 12", 0x83ccabdde1e0fdb7),
    ("sample 4 13", 0x6106931690663291),
    ("switch 4 on", 0x1595caa5cd6ca3ff),
    ("switch 4 off", 0xa1a6a837607e28b1),
    ("long 4", 0x11ebfc0d08d19ecc),
    ("empty 4", 0xd541f758263ceebb),
    ("sample 5 0", 0xfc8d09be45f119db),
    ("sample 5 1", 0x35623e4bdb668438),
    ("sample 5 2", 0xabc0c7ea1a614fa3),
    ("sample 5 3", 0xc48f1abcceff7281),
    ("sample 5 4", 0x2edf84bdaf60b9ec),
    ("sample 5 5", 0x95f8633692cb9228),
    ("sample 5 6", 0x3488ef30a68f3585),
    ("sample 5 7", 0x88183fcc9c575718),
    ("sample 5 8", 0x31c2f170a7bb9909),
    ("sample 5 9", 0xe42df48a5f5dfc3a),
    ("sample 5 10", 0xb2e5eee27f1b0e84),
    ("sample 5 11", 0x02c3ce45c282ce3d),
    ("sample 5 12", 0x0c9c8c219ce330d3),
    ("sample 5 13", 0xde4c5799d7ad5077),
    ("switch 5 on", 0xef63c3c474130705),
    ("switch 5 off", 0xc53227856ed80c16),
    ("long 5", 0x017123d2bf150b4e),
    ("empty 5", 0xd541f758263ceebb),
    ("picture Fit ", 0x51e5a860bb9af191),
    ("picture Fit Garden", 0x3d84c5d80a34d3ad),
    ("picture Fill ", 0x439e9ff791d870dd),
    ("picture Fill Garden", 0xc99b6ab82d984dd5),
    ("picture Stretch ", 0x7b1ce005d9d2b1a4),
    ("picture Stretch Garden", 0x889981f31a3664db),
    ("picture Original ", 0x9c9c7f72ff0bf858),
    ("picture Original Garden", 0x82858bea92a44ec0),
    ("camera [0, 0] [1, 1]", 0x4526ae8bf97f9ddf),
    ("camera [1, 0] [2, 2]", 0xe8a7a0a6ee5df1dc),
    ("camera [0, 1] [2, 2]", 0x842bbd6dd8dd48cc),
    ("band Bar CPU None - 0", 0x0445825b2519fe49),
    ("band Bar CPU None - 1", 0x0445825b2519fe49),
    ("band Bar CPU None - 2", 0xb0cf7cde286f6aeb),
    ("band Bar  Some(Gpu) - 0", 0x695babb0ecbc9e75),
    ("band Bar  Some(Gpu) - 1", 0x695babb0ecbc9e75),
    ("band Bar  Some(Gpu) - 2", 0x9ff30546efe788c7),
    ("band Bar  None mdi:thermometer 0", 0xcf2f921271e4dfd7),
    ("band Bar  None mdi:thermometer 1", 0xcf2f921271e4dfd7),
    ("band Bar  None mdi:thermometer 2", 0xe499ceaac538f0a0),
    ("band Blocks CPU None - 0", 0xd61ebd1b8520be91),
    ("band Blocks CPU None - 1", 0xd61ebd1b8520be91),
    ("band Blocks CPU None - 2", 0x2080083234054063),
    ("band Blocks  Some(Gpu) - 0", 0xab1b6e0b5843fbcd),
    ("band Blocks  Some(Gpu) - 1", 0xab1b6e0b5843fbcd),
    ("band Blocks  Some(Gpu) - 2", 0x5944a0d0a087541f),
    ("band Blocks  None mdi:thermometer 0", 0xee45a8f83ccc0d2f),
    ("band Blocks  None mdi:thermometer 1", 0xee45a8f83ccc0d2f),
    ("band Blocks  None mdi:thermometer 2", 0x4a2595f25208bbb0),
    ("band Gauge CPU None - 0", 0x2796f820b0220f31),
    ("band Gauge CPU None - 1", 0x2796f820b0220f31),
    ("band Gauge CPU None - 2", 0x460cf88f672749a6),
    ("band Gauge  Some(Gpu) - 0", 0x56db3483a0e8ae68),
    ("band Gauge  Some(Gpu) - 1", 0x56db3483a0e8ae68),
    ("band Gauge  Some(Gpu) - 2", 0x0f80183c9de23b8f),
    ("band Gauge  None mdi:thermometer 0", 0xb6675c687f087dae),
    ("band Gauge  None mdi:thermometer 1", 0xb6675c687f087dae),
    ("band Gauge  None mdi:thermometer 2", 0x2bced94cb51394d5),
    ("band Ring CPU None - 0", 0x35d00be2fccaaf7d),
    ("band Ring CPU None - 1", 0x35d00be2fccaaf7d),
    ("band Ring CPU None - 2", 0xb848ec1244452ef8),
    ("band Ring  Some(Gpu) - 0", 0x1f6e24461c9eae84),
    ("band Ring  Some(Gpu) - 1", 0x1f6e24461c9eae84),
    ("band Ring  Some(Gpu) - 2", 0xf828cb4f0f1811f5),
    ("band Ring  None mdi:thermometer 0", 0xb6cac4a9dd49dc22),
    ("band Ring  None mdi:thermometer 1", 0xb6cac4a9dd49dc22),
    ("band Ring  None mdi:thermometer 2", 0x658a1ce2bdec224b),
    ("band Graph CPU None - 0", 0x8d4a174bcc0a15ed),
    ("band Graph CPU None - 1", 0x8d4a174bcc0a15ed),
    ("band Graph CPU None - 2", 0x3c2c06372a1a269c),
    ("band Graph  Some(Gpu) - 0", 0x698107fb50c4e0a2),
    ("band Graph  Some(Gpu) - 1", 0x698107fb50c4e0a2),
    ("band Graph  Some(Gpu) - 2", 0x9f9318f20582fc09),
    ("band Graph  None mdi:thermometer 0", 0x64c7b72539d77f58),
    ("band Graph  None mdi:thermometer 1", 0x64c7b72539d77f58),
    ("band Graph  None mdi:thermometer 2", 0xa28cb6eaa80da697),
    ("band Number CPU None - 0", 0x360fed887764f0bf),
    ("band Number CPU None - 1", 0x360fed887764f0bf),
    ("band Number CPU None - 2", 0xd2d7778b4c3648c1),
    ("band Number  Some(Gpu) - 0", 0x9116a73f09a1ae74),
    ("band Number  Some(Gpu) - 1", 0x9116a73f09a1ae74),
    ("band Number  Some(Gpu) - 2", 0x5a621706a86e4b8f),
    ("band Number  None mdi:thermometer 0", 0xd53e5c989207e17e),
    ("band Number  None mdi:thermometer 1", 0xd53e5c989207e17e),
    ("band Number  None mdi:thermometer 2", 0x4860dbb84fad9de0),
    ("band icon Cpu", 0x80e196deaafaa74c),
    ("band icon Memory", 0xf63069a1b6097ba3),
    ("band text", 0x9faa239d2786345e),
    ("band picture", 0x45b8ad8b12fdddf9),
];

#[test]
fn pictures_at_the_usual_size_are_unchanged_to_the_byte() {
    let drawn = gallery(KEY_SIZE);
    assert_eq!(drawn.len(), GALLERY_AT_USUAL_SIZE.len());
    for ((name, picture), (expected_name, expected)) in drawn.iter().zip(GALLERY_AT_USUAL_SIZE) {
        assert_eq!(name, expected_name);
        assert_eq!(fingerprint(picture), *expected, "{name} looks different");
    }
}

/// How far apart two pictures of the same size are, on average per channel.
fn mean_difference(a: &RgbImage, b: &RgbImage) -> f32 {
    let total: u64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    total as f32 / a.as_raw().len() as f32
}

#[test]
fn a_picture_twice_the_size_is_laid_out_the_same() {
    let usual = gallery(KEY_SIZE);
    let doubled = gallery(2 * KEY_SIZE);
    let mut total = 0.0;
    for ((name, small), (_, big)) in usual.iter().zip(&doubled) {
        assert_eq!(big.width(), 2 * small.width(), "{name}");
        let shrunk = image::imageops::resize(big, small.width(), small.height(), FilterType::Triangle);
        let difference = mean_difference(small, &shrunk);
        // Text sits on whole pixels, so a big outlined title can land half a
        // pixel away; anything laid out differently is far more than this.
        assert!(difference < 12.5, "{name} differs by {difference:.2} on average");
        total += difference;
    }
    let average = total / usual.len() as f32;
    assert!(average < 4.0, "the pictures differ by {average:.2} on average");
}


#[test]
fn a_band_tile_can_have_its_own_background() {
    let face = Face { label: "CPU".into(), value: "12%".into(), ..Face::default() };
    let style = KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() };
    assert_eq!(band_picture(&face, &style).get_pixel(3, 76).0, BAND_BACKGROUND);
    let white = Face { background: Some([255, 255, 255]), ..face };
    assert_eq!(band_picture(&white, &style).get_pixel(3, 76).0, [255, 255, 255]);
}

fn restart_key() -> DockKey {
    let mut key = DockKey::default();
    crate::ui::add_library_action_for_tests(&mut key, HaAction::restart_home_assistant());
    key
}

#[test]
fn home_assistant_keys_say_when_it_cant_be_reached() {
    let style = KeyStyle::default();
    let down = Availability::Down { label: false, restarting: false };
    let ready = Look::of_with(&bound_key(), &style, &entities("on"), Availability::Ready).unwrap();
    assert!(!ready.unavailable);
    // Its last look, dimmed with a mark, and a picture of its own.
    let last = Look::of_with(&bound_key(), &style, &entities("on"), down).unwrap();
    assert!(last.unavailable);
    assert_eq!(last.text, ready.text);
    assert_ne!(key_picture(&last), key_picture(&ready));
    let brightness = |picture: &RgbImage| picture.pixels().map(|p| p.0.iter().map(|&c| c as u64).sum::<u64>()).sum::<u64>();
    assert!(brightness(&key_picture(&last)) < brightness(&key_picture(&ready)), "dimmed");
    // "HA offline", by choice or with nothing known about the entity.
    let label = Look::of_with(&bound_key(), &style, &entities("on"), Availability::Down { label: true, restarting: false }).unwrap();
    let unknown = Look::of_with(&bound_key(), &style, &Entities::new(), down).unwrap();
    for offline in [&label, &unknown] {
        assert_eq!((offline.text.as_str(), offline.glyph.as_str(), offline.unavailable), ("HA offline", OFFLINE_ICON, false));
    }
    // A key that doesn't touch Home Assistant isn't changed.
    let plain = DockKey { title: "Coffee".into(), ..DockKey::default() };
    assert_eq!(Look::of_with(&plain, &style, &Entities::new(), down), Look::of_with(&plain, &style, &Entities::new(), Availability::Ready));
}

#[test]
fn the_restart_key_shows_itself_and_then_restarting() {
    let style = KeyStyle::default();
    let key = restart_key();
    let normal = Look::of_with(&key, &style, &Entities::new(), Availability::Ready).unwrap();
    assert_eq!((normal.text.as_str(), normal.glyph.as_str()), ("Restart HA", "mdi:restart"));
    let down = Look::of_with(&key, &style, &Entities::new(), Availability::Down { label: false, restarting: false }).unwrap();
    assert!(down.unavailable, "no entity to know, but its own look, dimmed");
    let restarting = Look::of_with(&key, &style, &Entities::new(), Availability::Down { label: false, restarting: true }).unwrap();
    assert_eq!((restarting.text.as_str(), restarting.unavailable), ("Restarting…", false));
}

/// Pictures to look at: `cargo test -- --ignored offline_pictures`, then
/// target/work/offline-pictures.
#[test]
#[ignore]
fn offline_pictures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/work/offline-pictures");
    std::fs::create_dir_all(&dir).unwrap();
    let style = KeyStyle { shape: crate::model::KeyShape::Rounded, ..KeyStyle::default() };
    let switch = bound_key_with_switch();
    let cases: Vec<(&str, DockKey, Entities, Availability)> = vec![
        ("normal", switch.clone(), entities("on"), Availability::Ready),
        ("dimmed", switch.clone(), entities("on"), Availability::Down { label: false, restarting: false }),
        ("offline", switch, entities("on"), Availability::Down { label: true, restarting: false }),
        ("restart", restart_key(), Entities::new(), Availability::Ready),
        ("restarting", restart_key(), Entities::new(), Availability::Down { label: false, restarting: true }),
    ];
    for (name, key, entities, ha) in cases {
        key_picture_sized(&Look::of_with(&key, &style, &entities, ha).unwrap(), 170).save(dir.join(format!("{name}.png"))).unwrap();
    }
    let mut prompt = Look::of_with(&restart_key(), &style, &Entities::new(), Availability::Ready).unwrap();
    prompt.text = crate::confirm::PROMPT.into();
    prompt.color = crate::confirm::PROMPT_COLOR;
    prompt.style.color = [255, 255, 255];
    key_picture_sized(&prompt, 170).save(dir.join("press-again.png")).unwrap();
}
