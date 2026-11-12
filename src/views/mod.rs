//! How the screens draw, in both themes: the main window, the editor and the capture's bars,
//! from the state in `screens` (which carries out the actions they return). One layout serves
//! TakeUI and Coral UI; a theme changes the tokens, the mark and name, and the look of a few kit
//! components, never where a function is.

pub mod capture;
pub mod editor;
pub mod home;
pub mod settings;
pub mod shell;
pub mod style;

use crate::Shared;
use crate::engine::model::{PALETTE, Tool};
use crate::screens::home::{Home, MainAction};
use crate::ui::i18n::{Msg, t as tr};
use crate::ui::kit::{self, Ico};
use crate::ui::tokens::{ThemeId, Tokens};
use egui::{Color32, Ui, vec2};

/// The main window. Returns what the user asked for; `Home::apply` does it.
pub fn main_window(
    ui: &mut egui::Ui,
    t: &Tokens,
    home: &mut Home,
    shared: &mut Shared,
) -> Vec<MainAction> {
    let mut acts = home.begin(ui, shared);
    crate::ui::chrome::adopt_window("Takeshot");
    acts.extend(shell::main_window(ui, t, home, shared));
    acts
}

/// The editor. Returns what the user asked for; `Editor::apply` does it.
pub fn editor(
    ui: &mut egui::Ui,
    t: &Tokens,
    ed: &mut crate::screens::editor::Editor,
    shared: &mut Shared,
) -> Vec<crate::screens::editor::EditorAction> {
    editor::editor(ui, t, ed, shared)
}

/// The capture's bars and hint over `screen`. Returns what the user asked for; `Overlay::apply`
/// does it.
pub fn capture(
    ui: &mut egui::Ui,
    t: &Tokens,
    ov: &crate::screens::overlay::Overlay,
    shared: &Shared,
    screen: egui::Rect,
) -> Vec<crate::screens::overlay::CaptureAction> {
    capture::capture(ui, t, ov, shared, screen)
}

/// Applies the egui style for this look when it changed since the last frame (theme or light and
/// dark); true when it did.
pub fn begin_frame(ctx: &egui::Context, t: &Tokens) -> bool {
    let id = egui::Id::unique("takeshot-look");
    let look = (t.theme, t.dark);
    if ctx.data(|d| d.get_temp::<(ThemeId, bool)>(id)) == Some(look) {
        return false;
    }
    crate::ui::style::apply(ctx, t);
    ctx.data_mut(|d| d.insert_temp(id, look));
    true
}

/// The icon of a tool; `None` for the numbered step, which the kit draws as a circle with "1".
pub(crate) fn tool_icon(tool: Tool) -> Option<Ico> {
    Some(match tool {
        Tool::Select => Ico::Select,
        Tool::Pen => Ico::Pen,
        Tool::Line => Ico::Line,
        Tool::Arrow => Ico::Arrow,
        Tool::Rect => Ico::Rect,
        Tool::Ellipse => Ico::Ellipse,
        Tool::Marker => Ico::Marker,
        Tool::Text => Ico::Text,
        Tool::Blur => Ico::Blur,
        Tool::Counter => return None,
    })
}

/// A 1 px line `height` tall between groups of a bar, in the border colour.
pub(crate) fn divider(ui: &mut Ui, t: &Tokens, height: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, height), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
}

/// The palette's colours; returns the colour picked.
pub(crate) fn palette_dots(ui: &mut Ui, t: &Tokens, color: [u8; 4]) -> Option<[u8; 4]> {
    let mut picked = None;
    for c in PALETTE {
        let on = color[..3] == c[..3];
        let col = Color32::from_rgb(c[0], c[1], c[2]);
        let tip = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
        if kit::color_dot(ui, t, col, on, &tip).clicked() {
            picked = Some(c);
        }
    }
    picked
}

/// The palette's colours and the custom colour (a picker in a popup); returns the colour picked.
pub(crate) fn palette(ui: &mut Ui, t: &Tokens, color: [u8; 4]) -> Option<[u8; 4]> {
    let mut picked = palette_dots(ui, t, color);
    let add = kit::color_add(ui, t, tr(Msg::CustomColor));
    egui::Popup::from_toggle_button_response(&add)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut col = Color32::from_rgb(color[0], color[1], color[2]);
            if egui::color_picker::color_picker_color32(
                ui,
                &mut col,
                egui::color_picker::Alpha::Opaque,
            ) {
                picked = Some(col.to_array());
            }
        });
    picked
}

/// Headless frames over the main window, for the views' tests.
#[cfg(test)]
pub(crate) mod testing {
    use crate::Shared;
    use crate::screens::home::{Home, MainAction};
    use crate::ui::tokens::Tokens;
    use egui::{Color32, Event, Pos2, Rect};

    /// A painted rectangle and its fill.
    pub type Block = (Rect, Color32);

    /// The colour a mesh's block carries.
    pub const MESH: Color32 = Color32::PLACEHOLDER;

    /// What the last frame of a run painted, and the pointer it asked for.
    pub struct Painted {
        pub texts: Vec<(String, Rect)>,
        pub blocks: Vec<Block>,
        pub cursor: egui::CursorIcon,
    }

    /// Runs two warm-up frames (so widths remembered from a frame before settle), then one frame
    /// per event list at `size`, each drawn by `draw` in the theme's look; returns what the last
    /// one painted.
    fn run_frames(
        shared: &mut Shared,
        size: (f32, f32),
        dark: bool,
        frames: Vec<Vec<Event>>,
        mut draw: impl FnMut(&mut egui::Ui, &Tokens, &mut Shared),
    ) -> Painted {
        shared.settings.ui_mode = if dark {
            crate::ui::tokens::UiMode::Dark
        } else {
            crate::ui::tokens::UiMode::Light
        };
        let ctx = egui::Context::default();
        crate::system::fonts::FontLib::load().install(&ctx);
        let mut out = Painted {
            texts: Vec::new(),
            blocks: Vec::new(),
            cursor: egui::CursorIcon::Default,
        };
        for (i, events) in [vec![], vec![]].into_iter().chain(frames).enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(size.0, size.1))),
                time: Some(i as f64 * 0.05),
                events,
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                let t = shared.tokens(None);
                crate::views::begin_frame(ui.ctx(), &t);
                draw(ui, &t, shared);
            });
            full.textures_delta.clear();
            out.texts.clear();
            out.blocks.clear();
            for c in &full.shapes {
                walk(&c.shape, &mut out.texts, &mut out.blocks);
            }
            out.cursor = full.platform_output.cursor_icon;
        }
        out
    }

    /// The main window over `run_frames`; returns every action and the texts painted in the last
    /// frame with their screen rects.
    pub fn run(
        home: &mut Home,
        shared: &mut Shared,
        size: (f32, f32),
        dark: bool,
        frames: Vec<Vec<Event>>,
    ) -> (Vec<MainAction>, Vec<(String, Rect)>) {
        let (acts, painted, _) = run_painted(home, shared, size, dark, frames);
        (acts, painted)
    }

    /// `run`, plus the rectangles painted in the last frame.
    pub fn run_painted(
        home: &mut Home,
        shared: &mut Shared,
        size: (f32, f32),
        dark: bool,
        frames: Vec<Vec<Event>>,
    ) -> (Vec<MainAction>, Vec<(String, Rect)>, Vec<Block>) {
        let mut acts = Vec::new();
        let p = run_frames(shared, size, dark, frames, |ui, t, shared| {
            acts.extend(crate::views::main_window(ui, t, home, shared))
        });
        (acts, p.texts, p.blocks)
    }

    /// The editor window over `run_frames`; each frame's actions are carried out right away, as
    /// `Editor::ui` does.
    pub fn drive_editor(
        ed: &mut crate::screens::editor::Editor,
        shared: &mut Shared,
        size: (f32, f32),
        dark: bool,
        frames: Vec<Vec<Event>>,
    ) -> (Vec<crate::screens::editor::EditorAction>, Painted) {
        let mut acts = Vec::new();
        let p = run_frames(shared, size, dark, frames, |ui, t, shared| {
            let frame = crate::views::editor(ui, t, ed, shared);
            for a in &frame {
                ed.apply(a.clone(), ui.ctx(), None, shared);
            }
            acts.extend(frame);
        });
        (acts, p)
    }

    /// The capture's bars and hint over `run_frames`, the window as large as the sample picture;
    /// returns the actions without carrying them out (they reach the clipboard and the disk).
    pub fn run_capture(
        ov: &mut crate::screens::overlay::Overlay,
        shared: &mut Shared,
        frames: Vec<Vec<Event>>,
    ) -> (Vec<crate::screens::overlay::CaptureAction>, Painted) {
        let mut acts = Vec::new();
        let p = run_frames(shared, (1600.0, 900.0), true, frames, |ui, t, shared| {
            let screen = ui.max_rect();
            acts.extend(crate::views::capture(ui, t, ov, shared, screen));
        });
        (acts, p)
    }

    /// The whole capture frame by frame, through `Overlay::ui` (canvas, text field and bars), on
    /// the 1600 x 900 sample.
    pub fn run_overlay(
        ov: &mut crate::screens::overlay::Overlay,
        shared: &mut Shared,
        frames: Vec<Vec<Event>>,
    ) -> Painted {
        run_frames(shared, (1600.0, 900.0), true, frames, |ui, t, shared| {
            ov.ui(ui, None, t, shared);
        })
    }

    /// `drive_editor`, as actions, texts and blocks.
    pub fn run_editor(
        ed: &mut crate::screens::editor::Editor,
        shared: &mut Shared,
        size: (f32, f32),
        dark: bool,
        frames: Vec<Vec<Event>>,
    ) -> (
        Vec<crate::screens::editor::EditorAction>,
        Vec<(String, Rect)>,
        Vec<Block>,
    ) {
        let (acts, p) = drive_editor(ed, shared, size, dark, frames);
        (acts, p.texts, p.blocks)
    }

    fn walk(s: &egui::Shape, texts: &mut Vec<(String, Rect)>, blocks: &mut Vec<Block>) {
        match s {
            egui::Shape::Text(t) => texts.push((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            egui::Shape::Rect(r) => blocks.push((r.rect, r.fill)),
            // A line counts as a block of its stroke's colour; a mesh (a gradient, a picture) as
            // a block of `MESH`.
            egui::Shape::LineSegment { points, stroke } => {
                blocks.push((Rect::from_two_pos(points[0], points[1]), stroke.color))
            }
            egui::Shape::Mesh(m) => blocks.push((m.calc_bounds(), MESH)),
            // A circle (a colour or width dot) as the square around it, in its fill.
            egui::Shape::Circle(c) => blocks.push((
                Rect::from_center_size(c.center, egui::Vec2::splat(c.radius * 2.0)),
                c.fill,
            )),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts, blocks)),
            _ => {}
        }
    }

    /// The rounded box the pages sit in: the largest rectangle in the surface colour.
    pub fn content_box(blocks: &[Block], t: &Tokens) -> Rect {
        blocks
            .iter()
            .filter(|(_, c)| *c == t.surf)
            .map(|(r, _)| *r)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the content box")
    }

    /// The centre of the first painted text equal to one of `texts` whose rect passes `keep`.
    pub fn painted_at(
        painted: &[(String, Rect)],
        texts: &[&str],
        keep: impl Fn(Rect) -> bool,
    ) -> Option<Pos2> {
        painted
            .iter()
            .find(|(s, r)| texts.contains(&s.as_str()) && keep(*r))
            .map(|(_, r)| r.center())
    }

    fn button(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    pub fn click(p: Pos2) -> Vec<Vec<Event>> {
        vec![
            vec![Event::PointerMoved(p)],
            vec![button(p, true)],
            vec![button(p, false)],
        ]
    }

    /// A right click at `p`, then two frames for the menu it opens to settle.
    pub fn right_click(p: Pos2) -> Vec<Vec<Event>> {
        let b = |pressed| Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![
            vec![Event::PointerMoved(p)],
            vec![b(true)],
            vec![b(false)],
            vec![],
            vec![],
        ]
    }

    /// The width of the narrowest 32 px tall block under `p`: a menu row.
    pub fn menu_row_width(blocks: &[Block], p: Pos2) -> f32 {
        blocks
            .iter()
            .filter(|(r, _)| (r.height() - 32.0).abs() < 0.5 && r.contains(p))
            .map(|(r, _)| r.width())
            .fold(f32::INFINITY, f32::min)
    }

    pub fn double_click(p: Pos2) -> Vec<Vec<Event>> {
        vec![
            vec![Event::PointerMoved(p)],
            vec![button(p, true)],
            vec![button(p, false)],
            vec![button(p, true)],
            vec![button(p, false)],
        ]
    }
}

#[cfg(test)]
mod look_tests {
    use super::*;
    use crate::ui::tokens::{ThemeId, tokens};

    #[test]
    fn style_is_applied_once_per_look() {
        let ctx = egui::Context::default();
        assert!(
            begin_frame(&ctx, &tokens(ThemeId::TakeUi, true)),
            "first frame"
        );
        assert!(
            !begin_frame(&ctx, &tokens(ThemeId::TakeUi, true)),
            "same look"
        );
        assert!(
            begin_frame(&ctx, &tokens(ThemeId::TakeUi, false)),
            "light now"
        );
        assert!(
            begin_frame(&ctx, &tokens(ThemeId::Coral, false)),
            "other theme"
        );
    }
}
