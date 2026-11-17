//! Colours, themes and the small text helpers used everywhere in the window.

use super::*;

/// True while the window is showing its light theme.
pub(super) static LIGHT: AtomicBool = AtomicBool::new(false);

pub(super) const ACCENT: Color32 = Color32::from_rgb(79, 140, 255);

pub(super) fn themed(dark: (u8, u8, u8), light: (u8, u8, u8)) -> Color32 {
    let (r, g, b) = if LIGHT.load(Ordering::Relaxed) { light } else { dark };
    Color32::from_rgb(r, g, b)
}

pub(super) fn c_bg() -> Color32 {
    themed((18, 20, 26), (238, 240, 245))
}

pub(super) fn c_panel() -> Color32 {
    themed((25, 28, 36), (250, 251, 253))
}

pub(super) fn c_card() -> Color32 {
    themed((33, 37, 47), (230, 233, 240))
}

pub(super) fn c_line() -> Color32 {
    themed((46, 51, 63), (205, 209, 219))
}

pub(super) fn c_text() -> Color32 {
    themed((232, 234, 240), (28, 31, 38))
}

pub(super) fn c_dim() -> Color32 {
    themed((150, 156, 170), (92, 98, 112))
}

pub(super) fn c_warn() -> Color32 {
    themed((244, 164, 96), (176, 86, 12))
}

pub(super) fn c_good() -> Color32 {
    themed((110, 200, 150), (22, 122, 76))
}

pub(super) const SWATCHES: [[u8; 3]; 9] = [
    [0, 0, 0],
    [44, 49, 61],
    [70, 80, 100],
    [46, 104, 214],
    [32, 138, 104],
    [112, 78, 196],
    [196, 106, 44],
    [186, 58, 78],
    [176, 140, 30],
];

pub(super) fn visuals(light: bool) -> egui::Visuals {
    LIGHT.store(light, Ordering::Relaxed);
    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = c_bg();
    v.window_fill = c_panel();
    v.extreme_bg_color = themed((14, 16, 21), (255, 255, 255));
    v.override_text_color = Some(c_text());
    v.selection.bg_fill = if light { Color32::from_rgb(190, 214, 255) } else { ACCENT.gamma_multiply(0.55) };
    v.selection.stroke = Stroke::new(1.0, c_text());
    let radius = CornerRadius::same(7);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.noninteractive,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    v.widgets.inactive.weak_bg_fill = c_card();
    // Tick boxes use this, and they have to show on a card as well.
    v.widgets.inactive.bg_fill = themed((74, 80, 96), (186, 191, 203));
    v.widgets.hovered.weak_bg_fill = c_line();
    v.widgets.hovered.bg_fill = themed((96, 103, 120), (164, 170, 184));
    v.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.7);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c_line());
    v
}

pub fn apply_theme(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_visuals_of(Theme::Light, visuals(true));
    ctx.set_visuals_of(Theme::Dark, visuals(false));
    ctx.set_theme(match choice {
        ThemeChoice::Windows => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 7.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.0));
    });
}

pub(super) fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(c_dim())
}

pub(super) fn heading(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(RichText::new(text).size(12.0).color(c_dim()).strong())
}

pub(super) fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}
