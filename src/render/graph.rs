//! A sensor's recent past, drawn behind a key's text the way a band tile
//! draws its graph: a filled area with a brighter top edge, newest on the right.

use super::*;

/// The share of a key the graph's lowest and highest points reach, from the bottom.
const LOWEST: f32 = 0.08;
const HIGHEST: f32 = 0.62;

/// A key's graph: each point's height in percent of the graph's own range,
/// or none where nothing is known.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyGraph {
    pub points: Vec<Option<u8>>,
}

impl KeyGraph {
    /// Scales numbers between their own lowest and highest, as a key has no
    /// range of its own. A flat line sits halfway. Nothing known, no graph.
    pub fn of(values: &[Option<f32>]) -> Option<KeyGraph> {
        let known = values.iter().flatten().copied();
        let (low, high) = known.fold(None, |range: Option<(f32, f32)>, value| {
            Some(range.map_or((value, value), |(low, high)| (low.min(value), high.max(value))))
        })?;
        let points = values
            .iter()
            .map(|value| {
                let value = (*value)?;
                let share = if high - low > f32::EPSILON { (value - low) / (high - low) } else { 0.5 };
                Some((share * 100.0).round().clamp(0.0, 100.0) as u8)
            })
            .collect();
        Some(KeyGraph { points })
    }
}

/// Draws the graph across the whole key, from the bottom up. Each point
/// takes an equal share of the width, so it fits a key of any size.
pub(super) fn draw_key_graph(canvas: &mut RgbImage, graph: &KeyGraph, background: [u8; 3], scale: f32) {
    let count = graph.points.len();
    if count == 0 {
        return;
    }
    let (width, height) = (canvas.width() as i32, canvas.height() as f32);
    let color = visible_on(background, [255, 255, 255], SHAPE_CONTRAST);
    let edge = (2.0 * scale).round().max(1.0) as i32;
    let bottom = canvas.height() as i32;
    for x in 0..width {
        let point = (x as usize * count) / width as usize;
        let Some(percent) = graph.points[point.min(count - 1)] else { continue };
        let share = LOWEST + (HIGHEST - LOWEST) * percent as f32 / 100.0;
        let peak = bottom - (height * share).round() as i32;
        for y in peak..bottom {
            blend(canvas, x, y, color, if y < peak + edge { 0.75 } else { 0.22 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, DockKey, HaAction, HistorySpan};

    #[test]
    fn a_graph_spans_its_own_lowest_and_highest() {
        assert_eq!(KeyGraph::of(&[]), None);
        assert_eq!(KeyGraph::of(&[None, None]), None);
        let graph = KeyGraph::of(&[None, Some(10.0), Some(15.0), Some(20.0)]).unwrap();
        assert_eq!(graph.points, [None, Some(0), Some(50), Some(100)]);
        assert_eq!(KeyGraph::of(&[Some(3.0), Some(3.0)]).unwrap().points, [Some(50), Some(50)]);
    }

    fn graph_look(points: Vec<Option<u8>>) -> Look {
        Look {
            text: "21.5 °C".into(),
            glyph: String::new(),
            color: [44, 49, 61],
            image: None,
            image_changed: None,
            fit: PictureFit::Fit,
            style: KeyStyle { shape: KeyShape::Square, ..KeyStyle::default() },
            toggle: None,
            track: None,
            camera: None,
            graph: Some(KeyGraph { points }),
            unavailable: false,
        }
    }

    /// How much lighter than the background a column is, near the bottom and
    /// at a share of the height.
    fn lit(picture: &RgbImage, x_share: f32, y_share: f32) -> bool {
        let (x, y) = ((picture.width() as f32 * x_share) as u32, (picture.height() as f32 * y_share) as u32);
        picture.get_pixel(x, y)[0] > 70
    }

    #[test]
    fn a_graph_rises_to_each_point_at_any_size() {
        // Low on the left, high on the right, nothing known in the middle.
        let points: Vec<Option<u8>> = (0..64).map(|i| if (28..36).contains(&i) { None } else { Some(if i < 32 { 0 } else { 100 }) }).collect();
        let look = graph_look(points);
        for px in [KEY_SIZE, 2 * KEY_SIZE] {
            let picture = key_picture_sized(&look, px);
            assert!(lit(&picture, 0.05, 0.97) && lit(&picture, 0.95, 0.97), "{px}: both ends have something");
            assert!(!lit(&picture, 0.05, 0.6) && lit(&picture, 0.95, 0.45), "{px}: the right is higher");
            assert!(!lit(&picture, 0.5, 0.97), "{px}: a gap stays empty");
            assert!(!lit(&picture, 0.95, 0.3), "{px}: the graph stays under the top");
        }
        let small = key_picture_sized(&look, KEY_SIZE);
        let big = key_picture_sized(&look, 2 * KEY_SIZE);
        let shrunk = image::imageops::resize(&big, KEY_SIZE, KEY_SIZE, FilterType::Triangle);
        let difference: u64 = small.as_raw().iter().zip(shrunk.as_raw()).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        assert!((difference as f32 / small.as_raw().len() as f32) < 6.0, "twice the size, laid out the same");
    }

    #[test]
    fn a_key_with_a_graph_shows_the_recorded_past_instead_of_its_icon() {
        let entity = "sensor.graph_key_test";
        let mut entities = Entities::new();
        let state = json_entity(entity, "21.5");
        entities.insert(entity.into(), state);
        let binding = HaAction { entity: entity.into(), title: "{{state}}".into(), graph: Some(HistorySpan::Hour), ..Default::default() };
        let key = DockKey { actions: vec![Action::HomeAssistant(binding)], ..Default::default() };
        let style = KeyStyle::default();
        // Until the records arrive, the key looks as it always did.
        let before = Look::of(&key, &style, &entities).unwrap();
        assert!(before.graph.is_none() && !before.glyph.is_empty());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        crate::ha::history::put(entity, HistorySpan::Hour, vec![(now - 3000.0, Some(19.0)), (now - 1000.0, Some(23.0))]);
        let after = Look::of(&key, &style, &entities).unwrap();
        let graph = after.graph.clone().unwrap();
        assert_eq!(graph.points.len(), 64);
        assert_eq!((graph.points[0], graph.points[20], graph.points[63]), (None, Some(0), Some(100)));
        assert!(after.glyph.is_empty());
        assert_ne!(key_picture(&before), key_picture(&after));
    }

    fn json_entity(id: &str, state: &str) -> crate::ha::Entity {
        crate::ha::Entity { state: state.into(), name: id.into(), unit: "°C".into(), ..Default::default() }
    }
}
