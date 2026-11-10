//! Applies the theme tokens to egui: colours, 8 px corners, no hover expansion, type scale and thin scroll bars.

use crate::ui::tokens::Tokens;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Regular,
    Medium,
    Bold,
    Mono,
    Icons,
    Brand,
}

pub fn family(w: Weight) -> FontFamily {
    match w {
        Weight::Regular => FontFamily::Proportional,
        Weight::Medium => FontFamily::Name("medium".into()),
        Weight::Bold => FontFamily::Name("bold".into()),
        Weight::Mono => FontFamily::Name("mono".into()),
        Weight::Icons => FontFamily::Name("icons".into()),
        Weight::Brand => FontFamily::Name("brand".into()),
    }
}

pub fn font(size: f32, weight: Weight) -> FontId {
    FontId::new(size, family(weight))
}

pub fn apply(ctx: &egui::Context, t: &Tokens) {
    ctx.set_theme(if t.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let r = CornerRadius::same(t.radius);
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        *v = if t.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = t.shell;
        v.window_fill = t.surf;
        v.extreme_bg_color = t.input;
        v.faint_bg_color = t.raised;
        v.window_stroke = Stroke::new(1.0, t.border);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = r;
        let [pr, pg, pb, _] = t.primary.to_array();
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(pr, pg, pb, 89);
        v.selection.stroke = Stroke::new(1.0, t.primary);
        v.hyperlink_color = t.accent;
        v.text_cursor.stroke = Stroke::new(2.0, t.primary);
        let w = &mut v.widgets;
        for (st, fill) in [
            (&mut w.noninteractive, t.surf),
            (&mut w.inactive, t.raised),
            (&mut w.hovered, t.hover),
            (&mut w.active, t.press),
            (&mut w.open, t.hover),
        ] {
            st.bg_fill = fill;
            st.weak_bg_fill = fill;
            st.corner_radius = r;
            st.expansion = 0.0;
            st.fg_stroke = Stroke::new(1.0, t.text);
            st.bg_stroke = Stroke::NONE;
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);

        s.animation_time = 0.08;
        s.interaction.selectable_labels = false;
        // The spacing every window was measured with (egui's own is tighter).
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.slider_width = 150.0;
        // A solid bar keeps its own room beside the content, and hover only colours it.
        s.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 6.0,
            bar_inner_margin: 8.0,
            ..egui::style::ScrollStyle::solid()
        };
        s.text_styles = [
            (TextStyle::Body, font(13.0, Weight::Regular)),
            (TextStyle::Button, font(13.0, Weight::Medium)),
            (TextStyle::Small, font(12.0, Weight::Regular)),
            (TextStyle::Heading, font(22.0, Weight::Bold)),
            (TextStyle::Monospace, font(11.0, Weight::Mono)),
        ]
        .into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tokens::{ThemeId, tokens};
    use egui::Color32;

    #[test]
    fn the_style_keeps_the_app_s_spacing() {
        // The spacing the windows were measured with; tests and the app share it.
        let ctx = egui::Context::default();
        apply(&ctx, &tokens(ThemeId::TakeUi, true));
        let s = ctx.global_style().spacing.clone();
        assert_eq!(s.item_spacing, egui::vec2(8.0, 8.0));
        assert_eq!(s.button_padding, egui::vec2(10.0, 5.0));
        assert_eq!(s.interact_size.y, 26.0);
        assert_eq!(s.slider_width, 150.0);
    }

    #[test]
    fn hover_never_expands() {
        let ctx = egui::Context::default();
        apply(&ctx, &tokens(ThemeId::Coral, true));
        let w = ctx.global_style().visuals.widgets.clone();
        for st in [
            &w.noninteractive,
            &w.inactive,
            &w.hovered,
            &w.active,
            &w.open,
        ] {
            assert_eq!(st.expansion, 0.0);
            assert_eq!(st.corner_radius, egui::CornerRadius::same(8));
        }
        assert_eq!(ctx.global_style().animation_time, 0.08);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            Color32::from_hex("#111111").unwrap()
        );
        let scroll = ctx.global_style().spacing.scroll;
        assert_eq!(scroll.bar_width, 6.0);
        // A floating bar widens on hover and covers the content; a solid one keeps its room.
        assert!(!scroll.floating);
        assert_eq!(scroll.bar_inner_margin, 8.0);
    }

    #[test]
    fn a_label_keeps_the_arrow_pointer() {
        let ctx = egui::Context::default();
        apply(&ctx, &tokens(ThemeId::TakeUi, true));
        let mut pointer = egui::CursorIcon::Default;
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(egui::pos2(20.0, 12.0))],
            vec![],
        ] {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                ui.label("2026-11-10 12-04-59.png");
            });
            out.textures_delta.clear();
            pointer = out.platform_output.cursor_icon;
        }
        assert_eq!(pointer, egui::CursorIcon::Default);
    }

    #[test]
    fn widget_states_follow_the_tokens() {
        let ctx = egui::Context::default();
        let t = tokens(ThemeId::TakeUi, false);
        apply(&ctx, &t);
        let s = ctx.global_style();
        let w = &s.visuals.widgets;
        for st in [&w.inactive, &w.hovered, &w.active, &w.open] {
            assert_eq!(st.bg_stroke, egui::Stroke::NONE, "borda muda entre estados");
        }
        assert_eq!(w.inactive.bg_fill, t.raised);
        assert_eq!(w.hovered.bg_fill, t.hover);
        assert_eq!(w.active.bg_fill, t.press);
        assert_eq!(s.visuals.window_fill, t.surf);
        assert_eq!(s.visuals.extreme_bg_color, t.input);
        assert_eq!(s.visuals.selection.bg_fill.a(), 89);
        assert_eq!(s.text_styles[&egui::TextStyle::Heading].size, 22.0);
        assert_eq!(
            s.text_styles[&egui::TextStyle::Button].family,
            family(Weight::Medium)
        );
    }
}
