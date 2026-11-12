//! The capture's bars, as in the design's capture screens: the tools in a column beside the area, the
//! options of the tool or the selected object in a panel next to them, the actions (pin, save,
//! edit, copy, close) in a row under the area, and the hint before an area is chosen. Behaviour is
//! in `screens::overlay`.

use super::{divider, tool_icon};
use crate::Shared;
use crate::engine::model::{BlurMode, Tool, WIDTHS};
use crate::engine::placement::capture_bars;
use crate::screens::editor::{object_label, tool_name};
use crate::screens::overlay::{CaptureAction, Overlay};
use crate::ui::i18n::{Msg, t as tr};
use crate::ui::kit::{self, BtnKind, Ico};
use crate::ui::style::{Weight, font};
use crate::ui::tokens::Tokens;
use egui::{Color32, Frame, Rect, Ui, vec2};

/// The bars' look: the surface with a 1 px border, 12 px corners and a soft shadow.
fn bar_frame(t: &Tokens) -> Frame {
    Frame::new()
        .fill(t.surf)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(12)
        .shadow(egui::Shadow {
            offset: [0, 10],
            blur: 30,
            spread: 0,
            color: Color32::from_black_alpha(102),
        })
}

/// How far an object's handles reach past its bounds, as in the editor's object bar.
const HANDLE_REACH: f32 = 6.0;

/// The bars once an area is chosen, the hint before. Returns what the user asked for.
pub fn capture(
    ui: &mut Ui,
    t: &Tokens,
    ov: &Overlay,
    shared: &Shared,
    screen: Rect,
) -> Vec<CaptureAction> {
    let mut acts = Vec::new();
    match ov.area_on(screen) {
        None => hint(ui.ctx(), t),
        Some(area) if ov.bars_shown() => bars(ui.ctx(), t, ov, shared, area, screen, &mut acts),
        Some(_) => {}
    }
    acts
}

/// The keys before an area is chosen, centred 22 px under the top of the screen.
fn hint(ctx: &egui::Context, t: &Tokens) {
    egui::Area::new(egui::Id::unique("takeui-capture-hint"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 22.0))
        .show(ctx, |ui| {
            bar_frame(t)
                .inner_margin(egui::Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (key, what) in [
                            (tr(Msg::CapDrag), Msg::CapSelect),
                            ("Shift", Msg::CapSquare),
                            ("Ctrl+A", Msg::CapWholeScreen),
                            ("C", Msg::CapCopyColor),
                            ("Esc", Msg::CapLeave),
                        ] {
                            ui.label(
                                egui::RichText::new(key)
                                    .font(font(12.5, Weight::Medium))
                                    .color(t.text),
                            );
                            ui.label(
                                egui::RichText::new(tr(what))
                                    .font(font(12.5, Weight::Regular))
                                    .color(t.muted),
                            );
                            ui.add_space(6.0);
                        }
                    });
                });
        });
}

/// The tools, the options (with a drawing tool or a selected object) and the actions, placed by
/// `capture_bars` from their sizes in the last frame.
fn bars(
    ctx: &egui::Context,
    t: &Tokens,
    ov: &Overlay,
    shared: &Shared,
    area: Rect,
    screen: Rect,
    acts: &mut Vec<CaptureAction>,
) {
    let tools_id = egui::Id::unique("takeshot-capture-tools");
    let opts_id = egui::Id::unique("takeshot-capture-options");
    let actions_id = egui::Id::unique("takeshot-capture-actions");
    let size = |id: egui::Id, default: egui::Vec2| {
        ctx.data(|d| d.get_temp::<egui::Vec2>(id))
            .unwrap_or(default)
    };
    let bar_tool = ov.an.bar_tool();
    // The selected object with the reach of its handles: the bars keep off it.
    let selected = ov
        .selection_on_screen(screen, shared)
        .map(|r| r.expand(HANDLE_REACH));
    let at = capture_bars(
        area,
        size(tools_id, vec2(46.0, 450.0)),
        bar_tool.map(|_| size(opts_id, vec2(222.0, 240.0))),
        size(actions_id, vec2(380.0, 42.0)),
        screen,
        selected,
    );
    let part = |id: egui::Id, at: Rect, margin: i8, body: &mut dyn FnMut(&mut Ui)| {
        let shown = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(at.min)
            .show(ctx, |ui| {
                bar_frame(t).inner_margin(margin).show(ui, |ui| body(ui))
            });
        // The frame as drawn: the area's own rect can stay wider than its content.
        ctx.data_mut(|d| d.insert_temp(id, shown.inner.response.rect.size()));
    };
    part(tools_id, at.tools, 5, &mut |ui| tools_bar(ui, t, ov, acts));
    if let (Some(tool), Some(o)) = (bar_tool, at.options) {
        part(opts_id, o, 12, &mut |ui| {
            options_panel(ui, t, ov, shared, tool, acts)
        });
    }
    part(actions_id, at.actions, 4, &mut |ui| {
        actions_bar(ui, t, acts)
    });
}

/// The tools in one column, then undo and redo; 34 px buttons 2 apart.
fn tools_bar(ui: &mut Ui, t: &Tokens, ov: &Overlay, acts: &mut Vec<CaptureAction>) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for tool in [Tool::Select].into_iter().chain(Tool::DRAWING) {
            let tip = format!("{} ({})", tr(tool_name(tool)), tool.key_name());
            if kit::tool_button(ui, t, tool_icon(tool), ov.an.tool == tool, &tip).clicked() {
                acts.push(CaptureAction::SetTool(tool));
            }
        }
        ui.add_space(3.0);
        let (line, _) = ui.allocate_exact_size(vec2(24.0, 1.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.border);
        ui.add_space(3.0);
        let keys = |name: Msg, k: &str| format!("{} ({k})", tr(name));
        for (icon, tip, act) in [
            (Ico::Undo, keys(Msg::Undo, "Ctrl+Z"), CaptureAction::Undo),
            (Ico::Redo, keys(Msg::Redo, "Ctrl+Y"), CaptureAction::Redo),
        ] {
            if kit::tool_button(ui, t, Some(icon), false, &tip).clicked() {
                acts.push(act);
            }
        }
    });
}

/// Pin, Save and Edit with their names, Copy (primary) and close, in one row.
fn actions_bar(ui: &mut Ui, t: &Tokens, acts: &mut Vec<CaptureAction>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let keys = |name: Msg, k: &str| format!("{} ({k})", tr(name));
        for (icon, label, tip, act) in [
            (
                Ico::Pin,
                Msg::PinOnScreen,
                keys(Msg::PinOnScreen, "Ctrl+T"),
                CaptureAction::Pin,
            ),
            (
                Ico::Download,
                Msg::Save,
                keys(Msg::Save, "Ctrl+S"),
                CaptureAction::Save,
            ),
            (
                Ico::Edit,
                Msg::Edit,
                keys(Msg::EditTip, "Enter"),
                CaptureAction::Edit,
            ),
        ] {
            if kit::bar_text(ui, t, icon, tr(label), false, false, &tip).clicked() {
                acts.push(act);
            }
        }
        if kit::button(
            ui,
            t,
            BtnKind::Primary,
            Some(Ico::Copy),
            tr(Msg::Copy),
            None,
        )
        .on_hover_text(keys(Msg::Copy, "Ctrl+C"))
        .clicked()
        {
            acts.push(CaptureAction::Copy);
        }
        divider(ui, t, 20.0);
        let close = format!("{} (Esc)", tr(Msg::CloseWord));
        if kit::bar_button(ui, t, kit::BarItem::Icon(Ico::Close), &close).clicked() {
            acts.push(CaptureAction::Close);
        }
    });
}

/// A slider's drag is one undo step: grabbed, it opens a gesture; let go, it closes it.
fn gesture(
    r: egui::Response,
    v: f32,
    acts: &mut Vec<CaptureAction>,
    set: impl Fn(f32) -> CaptureAction,
) {
    if r.drag_started() {
        acts.push(CaptureAction::Gesture(true));
    }
    if r.changed() {
        acts.push(set(v));
    }
    if r.drag_stopped() {
        acts.push(CaptureAction::Gesture(false));
    }
}

/// The tool's or the selected object's name, then its options, one under the other: colours,
/// widths, Corners for rectangles, Fill for rectangles and ellipses, font and background for
/// text, Blur or Pixelate for blur; with an object selected, Duplicate and Delete.
fn options_panel(
    ui: &mut Ui,
    t: &Tokens,
    ov: &Overlay,
    shared: &Shared,
    tool: Tool,
    acts: &mut Vec<CaptureAction>,
) {
    let opts = &shared.settings.draw;
    let doc = &ov.an.doc;
    let name = match doc.selected() {
        [one] => doc
            .kind(*one)
            .map(|(k, text)| object_label(k, text.as_deref()))
            .unwrap_or_default(),
        _ => tr(tool_name(tool)).to_string(),
    };
    ui.set_width(198.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    let mut job = egui::text::LayoutJob::simple_singleline(name, font(13.0, Weight::Bold), t.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(198.0);
    ui.label(job);
    if tool != Tool::Blur {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if let Some(c) = super::palette(ui, t, opts.color) {
                if ui.input(|i| i.pointer.any_down()) {
                    acts.push(CaptureAction::Gesture(true));
                }
                acts.push(CaptureAction::SetColor(c));
            }
        });
    }
    let small = |ui: &mut Ui, m: Msg| {
        ui.label(
            egui::RichText::new(tr(m))
                .font(font(12.0, Weight::Regular))
                .color(t.muted),
        );
    };
    match tool {
        Tool::Text => {
            let names: Vec<(usize, &str)> = shared
                .fonts
                .list
                .iter()
                .enumerate()
                .map(|(i, f)| (i, f.name.as_str()))
                .collect();
            let mut f = opts.font;
            if kit::select(ui, t, "takeshot-capture-font", &mut f, &names, 198.0).changed() {
                acts.push(CaptureAction::SetFont(f));
            }
            let size = opts.font_size.round();
            kit::combo(
                ui,
                t,
                "takeshot-capture-size",
                &format!("{size} px"),
                198.0,
                28.0,
                |ui| {
                    for s in [12.0, 16.0, 20.0, 24.0, 32.0, 48.0, 64.0, 96.0] {
                        if ui.selectable_label(size == s, format!("{s} px")).clicked() {
                            acts.push(CaptureAction::SetFontSize(s));
                        }
                    }
                },
            );
            let mut boxed = opts.text_box;
            if kit::checkbox(ui, t, &mut boxed, tr(Msg::TextBackground)).clicked() {
                acts.push(CaptureAction::SetTextBox(boxed));
            }
        }
        Tool::Blur => {
            let mut m = opts.blur_mode;
            let items = [
                (BlurMode::Blur, tr(Msg::ToolBlur)),
                (BlurMode::Pixelate, tr(Msg::Pixelate)),
            ];
            if kit::segmented(ui, t, &mut m, &items).changed() {
                acts.push(CaptureAction::SetBlurMode(m));
            }
        }
        _ => {
            small(ui, Msg::Width);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for w in WIDTHS {
                    let tip = format!("{w} px");
                    if kit::width_dot(ui, t, w, (opts.width - w).abs() < 0.01, &tip).clicked() {
                        acts.push(CaptureAction::SetWidth(w));
                    }
                }
            });
        }
    }
    if tool == Tool::Rect {
        let mut v = opts.radius;
        let r = kit::slider_row(ui, t, tr(Msg::Corners), &mut v, 0.0..=40.0, "px");
        gesture(r, v, acts, |v| CaptureAction::SetRadius(v.round()));
    }
    if matches!(tool, Tool::Rect | Tool::Ellipse) {
        let mut fill = opts.filled;
        if kit::checkbox(ui, t, &mut fill, tr(Msg::Fill)).clicked() {
            acts.push(CaptureAction::SetFill(fill));
        }
    }
    if !doc.selected().is_empty() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (icon, tip, act) in [
                (
                    Ico::CopyPlus,
                    format!("{} (Ctrl+D)", tr(Msg::Duplicate)),
                    CaptureAction::Duplicate,
                ),
                (
                    Ico::Trash,
                    format!("{} (Del)", tr(Msg::Delete)),
                    CaptureAction::Delete,
                ),
            ] {
                if kit::bar_button(ui, t, kit::BarItem::Icon(icon), &tip).clicked() {
                    acts.push(act);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::doc::Doc;
    use crate::engine::model::{Ann, BlurMode, Shape, Tool};
    use crate::screens::editor::tool_name;
    use crate::screens::overlay::{CaptureAction, Overlay};
    use crate::ui::i18n::{Lang, Msg, lang, set_lang, t as tr};
    use crate::views::testing::{click, run_capture};
    use egui::{Rect, pos2};

    /// The 1600 x 900 sample with the area (100,100)-(900,700) and a filled rectangle in it.
    fn capture(tool: Tool) -> (Overlay, crate::Shared) {
        let shared = crate::Shared::for_tests();
        let ov = Overlay::new(
            &egui::Context::default(),
            crate::system::capture::sample_shot(),
            &shared,
        )
        .with_area(Rect::from_min_max(pos2(100.0, 100.0), pos2(900.0, 700.0)))
        .with_doc(
            tool,
            Doc::from_anns(vec![Ann {
                shape: Shape::Rect(pos2(300.0, 300.0), pos2(400.0, 400.0), 0.0),
                color: [255, 0, 0, 255],
                width: 4.0,
                filled: true,
            }]),
        );
        (ov, shared)
    }

    fn tool_names() -> Vec<&'static str> {
        [Tool::Select]
            .into_iter()
            .chain(Tool::DRAWING)
            .map(|t| tr(tool_name(t)))
            .collect()
    }

    #[test]
    fn the_options_bar_shows_only_with_a_drawing_tool_or_a_selection() {
        let (mut ov, mut shared) = capture(Tool::Select);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        let names = tool_names();
        assert!(
            !p.texts.iter().any(|(s, _)| names.contains(&s.as_str())),
            "no options bar with Select and nothing selected: {:?}",
            p.texts
        );
        // A drawing tool: its name heads the options bar.
        let (mut ov, mut shared) = capture(Tool::Ellipse);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        assert!(p.texts.iter().any(|(s, _)| s == tr(Msg::ToolEllipse)));
        // A selected object: its name, and Duplicate on the options bar.
        let (mut ov, mut shared) = capture(Tool::Select);
        let id = ov.an.doc.objs()[0].id;
        ov.an.doc.select(Some(id), false);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        assert!(p.texts.iter().any(|(s, _)| s == tr(Msg::ToolRect)));
        let dup = crate::ui::icon::glyph(crate::ui::kit::Ico::CopyPlus).to_string();
        assert!(p.texts.iter().any(|(s, _)| *s == dup));
    }

    #[test]
    fn the_options_bar_offers_a_custom_colour_and_no_object_buttons_without_a_selection() {
        let (mut ov, mut shared) = capture(Tool::Pen);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        let glyph = |i| crate::ui::icon::glyph(i).to_string();
        assert!(
            p.texts
                .iter()
                .any(|(s, _)| *s == glyph(crate::ui::kit::Ico::Plus))
        );
        for i in [crate::ui::kit::Ico::CopyPlus, crate::ui::kit::Ico::Trash] {
            assert!(!p.texts.iter().any(|(s, _)| *s == glyph(i)), "{i:?}");
        }
    }

    #[test]
    fn edit_and_copy_buttons_act() {
        let (mut ov, mut shared) = capture(Tool::Select);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        let edit = p
            .texts
            .iter()
            .find(|(s, _)| s == tr(Msg::Edit))
            .map(|(_, r)| r.center())
            .expect("Edit");
        let (acts, _) = run_capture(&mut ov, &mut shared, click(edit));
        assert!(acts.contains(&CaptureAction::Edit), "{acts:?}");
        let copy = crate::ui::icon::glyph(crate::ui::kit::Ico::Copy).to_string();
        let at = p
            .texts
            .iter()
            .find(|(s, _)| *s == copy)
            .map(|(_, r)| r.center())
            .expect("Copy");
        let (acts, _) = run_capture(&mut ov, &mut shared, click(at));
        assert!(acts.contains(&CaptureAction::Copy), "{acts:?}");
    }

    #[test]
    fn the_tools_stand_in_a_column_beside_the_area() {
        // The area is (100,100)-(900,700) on a 1600 x 900 screen.
        let (mut ov, mut shared) = capture(Tool::Select);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        let glyph = |t: Tool| {
            let g = crate::ui::icon::glyph(crate::views::tool_icon(t).unwrap()).to_string();
            p.texts
                .iter()
                .find(|(s, _)| *s == g)
                .map(|(_, r)| r.center())
                .unwrap_or_else(|| panic!("{t:?}"))
        };
        let (select, pen, blur) = (glyph(Tool::Select), glyph(Tool::Pen), glyph(Tool::Blur));
        assert!(select.x > 900.0, "right of the area: {select:?}");
        assert!((select.x - blur.x).abs() < 0.5 && select.y < pen.y && pen.y < blur.y);
        assert!(select.y < 100.0 + 40.0, "from the area's top: {select:?}");
    }

    #[test]
    fn the_actions_end_at_the_area_s_right_edge() {
        let (mut ov, mut shared) = capture(Tool::Select);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        for m in [Msg::PinOnScreen, Msg::Save, Msg::Edit, Msg::Copy] {
            let r = p
                .texts
                .iter()
                .find(|(s, _)| s == tr(m))
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("{m:?}"));
            assert!(
                r.top() > 700.0 && r.right() < 900.0,
                "{m:?} under the area: {r:?}"
            );
        }
    }

    #[test]
    fn a_rectangle_offers_its_corners_in_the_capture() {
        let (mut ov, mut shared) = capture(Tool::Rect);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        assert!(
            p.texts.iter().any(|(s, _)| s == tr(Msg::Corners)),
            "{:?}",
            p.texts
        );
        let (mut ov, mut shared) = capture(Tool::Ellipse);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        assert!(
            !p.texts.iter().any(|(s, _)| s == tr(Msg::Corners)),
            "not for an ellipse"
        );
    }

    #[test]
    fn the_options_panel_keeps_its_width() {
        // 198 px inside 12 px margins: the colours wrap in rows instead of widening it.
        let (mut ov, mut shared) = capture(Tool::Arrow);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        let name = p
            .texts
            .iter()
            .find(|(s, _)| s == tr(Msg::ToolArrow))
            .map(|(_, r)| *r)
            .expect("the panel's title");
        // The smallest block around the title that is bigger than a control (the frame fades in,
        // so its colour is not yet the surface).
        let panel = p
            .blocks
            .iter()
            .filter(|(r, _)| r.contains(name.center()) && r.width() > 120.0 && r.height() > 120.0)
            .map(|(r, _)| *r)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the panel");
        assert!(panel.width() <= 224.0, "{panel:?}");
    }

    #[test]
    fn a_wide_area_keeps_the_tools_and_options_outside() {
        // 230 px left on each side: the tools go right, the options left, neither over the area.
        let (ov, mut shared) = capture(Tool::Arrow);
        let area = Rect::from_min_max(pos2(250.0, 100.0), pos2(1350.0, 700.0));
        let mut ov = ov.with_area(area);
        let (_, p) = run_capture(&mut ov, &mut shared, vec![vec![], vec![]]);
        let at = |s: &str| {
            p.texts
                .iter()
                .find(|(x, _)| x == s)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("{s}"))
        };
        let select =
            at(&crate::ui::icon::glyph(crate::views::tool_icon(Tool::Select).unwrap()).to_string());
        assert!(
            select.left() > area.right(),
            "tools right of the area: {select:?}"
        );
        let title = at(tr(Msg::ToolArrow));
        assert!(
            title.right() < area.left(),
            "options left of the area: {title:?}"
        );
    }

    /// A whole-screen capture (Ctrl+A) of the 1600 x 900 sample with a blur just drawn in the top
    /// right corner, selected as `Doc::add` leaves it; returns the blur's rect too.
    fn corner_blur() -> (Overlay, crate::Shared, Rect) {
        let shape = Rect::from_min_max(pos2(1330.0, 20.0), pos2(1580.0, 140.0));
        let shared = crate::Shared::for_tests();
        let mut ov = Overlay::new(
            &egui::Context::default(),
            crate::system::capture::sample_shot(),
            &shared,
        )
        .with_area(Rect::from_min_max(pos2(0.0, 0.0), pos2(1600.0, 900.0)))
        .with_doc(Tool::Blur, Doc::new());
        ov.an.doc.add(Ann {
            shape: Shape::Blur(shape.min, shape.max, BlurMode::Blur),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: false,
        });
        (ov, shared, shape)
    }

    #[test]
    fn a_whole_screen_capture_keeps_its_bars_off_a_shape_in_the_corner() {
        let (mut ov, mut shared, shape) = corner_blur();
        let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
        // The shape's selection marks reach a few px past it.
        let marked = shape.expand(6.0);
        for (s, r) in &p.texts {
            assert!(!r.intersects(marked), "{s:?} at {r:?} over the shape");
        }
        // The options keep to the screen, under the shape (the name heads the panel).
        let title = p
            .texts
            .iter()
            .filter(|(s, _)| s == tr(Msg::ToolBlur))
            .map(|(_, r)| *r)
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("the options panel's title");
        assert!(
            title.top() >= shape.bottom() && title.bottom() <= 900.0 - 8.0,
            "{title:?} under {shape:?}"
        );
    }

    #[test]
    fn capture_texts_follow_the_language() {
        let before = lang();
        for (l, edit, drag) in [(Lang::Pt, "Editar", "Arraste"), (Lang::En, "Edit", "Drag")] {
            set_lang(l);
            let (mut ov, mut shared) = capture(Tool::Select);
            let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
            assert!(
                p.texts.iter().any(|(s, _)| s == edit),
                "{l:?}: {:?}",
                p.texts
            );
            // Before an area is chosen, the hint.
            let mut ov = Overlay::new(
                &egui::Context::default(),
                crate::system::capture::sample_shot(),
                &shared,
            );
            let (_, p) = run_capture(&mut ov, &mut shared, vec![]);
            assert!(
                p.texts.iter().any(|(s, _)| s == drag),
                "{l:?}: {:?}",
                p.texts
            );
        }
        set_lang(before);
    }
}
