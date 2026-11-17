use super::*;

#[test]
fn a_pad_on_screen_resizes_from_any_edge_keeping_its_shape_and_the_far_side() {
    use super::virtual_pad::{resized, PadResize};
    use egui::ResizeDirection::*;
    let start = Rect::from_min_size(egui::pos2(100.0, 100.0), vec2(400.0, 200.0));
    // A made-up pad twice as wide as it is tall, keys of 80 at that size.
    let size_of = |tile: f32| vec2(tile * 5.0, tile * 2.5);
    let drag = |direction, by: Vec2| {
        let resize = PadResize { pad_id: "0123456789abcdef".into(), direction, from: start.center(), start, tile: 80.0 };
        resized(&resize, start.center() + by, size_of)
    };
    // Wider from the right: the left stays, and it grows in both directions.
    let right = drag(East, vec2(100.0, 0.0));
    assert_eq!((right.min, right.width()), (start.min, 500.0));
    assert_eq!(right.height(), 250.0);
    // Wider from the left: the right side stays put.
    let left = drag(West, vec2(-100.0, 0.0));
    assert_eq!(left.max.x, start.max.x);
    // From the top left corner: the bottom right stays put.
    let corner = drag(NorthWest, vec2(-40.0, -40.0));
    assert_eq!(corner.max, start.max);
    // Never smaller than keys of 36.
    assert_eq!(drag(East, vec2(-1000.0, 0.0)).width(), 180.0);
}

#[test]
fn shift_takes_the_rectangle_between_two_keys() {
    assert_eq!(cells_between(5, 1, 8), [1, 2, 3, 6, 7, 8]);
    assert_eq!(cells_between(5, 8, 1), [1, 2, 3, 6, 7, 8]);
    assert_eq!(cells_between(5, 4, 4), [4]);
}

#[test]
fn a_group_keeps_its_layout_when_copied_and_pasted() {
    let key = |title: &str| DockKey { title: title.into(), ..DockKey::default() };
    let mut keys = vec![DockKey::default(); 15];
    keys[6] = key("a");
    keys[8] = key("b");
    keys[13] = key("c");
    let clip = clip_of(&keys, &[6, 7, 8, 13], 5);
    let places: Vec<(usize, usize, &str)> = clip.iter().map(|(r, c, k)| (*r, *c, k.title.as_str())).collect();
    assert_eq!(places, [(0, 0, "a"), (0, 2, "b"), (1, 2, "c")]);
    // Pasted at the top left, all fit; at the far right, only what fits lands.
    assert_eq!(paste_cells(&clip, 0, 3, 5), [(0, 0), (1, 2), (2, 7)]);
    assert_eq!(paste_cells(&clip, 3, 3, 5), [(0, 3)]);
    assert_eq!(paste_cells(&clip, 10, 3, 5), [(0, 10), (1, 12)]);
}

#[test]
fn the_nearest_cell_is_found_even_over_a_gap() {
    let cell = |x, y| cell_at(vec2(x, y), 90.0, 12.0, 3, 5);
    assert_eq!(cell(10.0, 10.0), 0);
    assert_eq!(cell(100.0, 10.0), 1);
    assert_eq!(cell(92.0, 10.0), 0);
    assert_eq!(cell(97.0, 10.0), 1);
    assert_eq!(cell(10.0, 100.0), 5);
    assert_eq!(cell(500.0, 300.0), 14);
    assert_eq!(cell(-5.0, -5.0), 0);
}

#[test]
fn a_slide_slows_down_and_arrives() {
    let wanted = vec2(100.0, 0.0);
    let mut now = Vec2::ZERO;
    let mut last_step = f32::MAX;
    for _ in 0..60 {
        let next = glide_toward(now, wanted, 1.0 / 60.0);
        let step = (next - now).length();
        // The very last bit is an invisible snap into place.
        if next != wanted {
            assert!(step <= last_step);
        }
        last_step = step;
        now = next;
    }
    assert_eq!(now, wanted);
}

#[test]
fn slides_run_at_the_same_pace_on_any_screen() {
    let fast = (0..12).fold(Vec2::ZERO, |now, _| glide_toward(now, vec2(100.0, 0.0), 1.0 / 120.0));
    let slow = (0..6).fold(Vec2::ZERO, |now, _| glide_toward(now, vec2(100.0, 0.0), 1.0 / 60.0));
    assert!((fast - slow).length() < 0.5);
}

#[test]
fn swapping_keys_keeps_each_where_it_is_on_screen() {
    // Cell 0 sits at x 0 and cell 2 at x 200. The dragged key from cell 0
    // is on screen at 190; the other is on its way, at 130.
    let (from, to, shift) = (0, 2, vec2(-200.0, 0.0));
    let mut glide = HashMap::from([(from, vec2(190.0, 0.0)), (to, vec2(-70.0, 0.0))]);
    swap_glides(&mut glide, from, to, shift);
    assert_eq!(glide[&to], vec2(-10.0, 0.0));
    assert_eq!(glide[&from], vec2(130.0, 0.0));
}

#[test]
fn the_akp03e_layout_matches_the_pad_drawn_in_the_window() {
    use crate::model::layout::layout_for;
    // At 85-pixel keys the window's captions are a fifth of a key, as the layout takes them.
    let (tile, gaps) = (85.0, [35u8, 35]);
    let gap = tile * gaps[0] as f32 / 100.0;
    let device = Rect::from_min_size(egui::Pos2::ZERO, SmallPlan::device_size(tile, gap, 0.0));
    let plan = SmallPlan::new(device, tile, gap, 0.0);
    let layout = layout_for(PadKind::Akp03e, gaps, &crate::model::Profile::for_small_pad("Small".into()));
    assert!((layout.width * tile - device.width()).abs() < 0.5, "{} against {}", layout.width * tile, device.width());
    assert!((layout.height * tile - device.height()).abs() < 0.5);
    for control in &layout.controls {
        let cell = control.cell.unwrap();
        let drawn = match small_knob_of_cell(cell) {
            // A knob's parts share the whole knob's place in the layout.
            Some(knob) => Rect::from_center_size(plan.knobs[knob].0, Vec2::splat(plan.knobs[knob].1 * 2.0)),
            None => plan.zone(cell),
        };
        let laid = Rect::from_min_size(egui::pos2(control.x, control.y) * tile, vec2(control.w, control.h) * tile);
        assert!((laid.center() - drawn.center()).length() < 0.5, "{}: {laid:?} against {drawn:?}", control.name);
        assert!((laid.size() - drawn.size()).length() < 0.5, "{}", control.name);
    }
}

#[test]
fn a_dashboard_pad_is_a_grid_with_no_band_or_knobs() {
    let layout = PadLayout::scaled(PadKind::Dashboard, 2, 4, 80.0, egui::Pos2::ZERO);
    assert!(layout.plan.is_none());
    let main = PadLayout::scaled(PadKind::Main, 2, 4, 80.0, egui::Pos2::ZERO);
    // The same keys, without the band's narrower column.
    assert!((main.device.width() - layout.device.width() - 80.0 * (crate::model::layout::BAND_TILE + 0.25)).abs() < 0.5);
    assert_eq!(layout.device.height(), main.device.height());
    assert_eq!(layout.cell_at(layout.home(7).center(), 2), Some(7));
}
