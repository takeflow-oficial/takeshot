//! Início: the capture buttons, the gallery filters, search and folder, and the grid of
//! captures from the save folder, as in "Início · escuro" and "Início · claro" of the design.

use crate::Shared;
use crate::screens::home::{Home, MainAction, when_label};
use crate::system::hotkeys::{Shot, label};
use crate::system::library::Period;
use crate::system::settings::HomeView;
use crate::ui::i18n::{Msg, t as tr, tf};
use crate::ui::kit::{self, BtnKind, CARD_TEXT_H, CardAction, Ico, MenuRow};
use crate::ui::style::{Weight, font};
use crate::ui::tokens::Tokens;
use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};

/// The narrowest card and the gap between cards (16 in the design, which fits five 228 px cards
/// in its 1280 px window).
const CARD_MIN: f32 = 200.0;
const GAP: f32 = 16.0;

/// Columns and card size for a grid `width` wide: pictures in 16:10 over the two text lines.
pub fn grid(width: f32) -> (usize, egui::Vec2) {
    // A layout pass can offer next to no room; a card is never narrower than the minimum.
    let width = width.max(CARD_MIN);
    let cols = (((width + GAP) / (CARD_MIN + GAP)).floor() as usize).max(1);
    let w = (width - GAP * (cols - 1) as f32) / cols as f32;
    (cols, vec2(w, (w - 12.0) * 10.0 / 16.0 + 6.0 + CARD_TEXT_H))
}

pub fn start_page(
    ui: &mut Ui,
    t: &Tokens,
    home: &mut Home,
    shared: &mut Shared,
    acts: &mut Vec<MainAction>,
) {
    let area = label(&shared.settings.hotkey_area);
    let full = label(&shared.settings.hotkey_full);
    // Measures from "Início · escuro": the header 24 px in and 22 from the top over an 87 px
    // band, a line across the box, the filters in a 56 px band 20 px in, the grid 24 px in.
    let inner = ui.max_rect();
    let b = inner.expand(1.0); // the box, its border included
    let head = ui
        .scope_builder(
            UiBuilder::new().max_rect(Rect::from_min_max(
                pos2(b.left() + 24.0, b.top() + 22.0),
                pos2(b.right() - 24.0, inner.bottom()),
            )),
            |ui| header(ui, t, &area, &full, acts),
        )
        .response
        .rect;
    let line_y = (b.top() + 87.0).max(head.bottom() + 18.0);
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(inner.left(), line_y),
            pos2(inner.right(), line_y + 1.0),
        ),
        0.0,
        t.border,
    );
    let filters = Rect::from_min_max(
        pos2(b.left() + 20.0, line_y + 1.0),
        pos2(b.right() - 20.0, line_y + 57.0),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(filters)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            filter_row(
                ui,
                t,
                home,
                shared.settings.home_view,
                shared.settings.home_details,
                acts,
            )
        },
    );
    // The detail panel takes the right side down to the box's bottom, behind a line.
    let details = shared.settings.home_details;
    let panel = Rect::from_min_max(
        pos2(b.right() - PANEL_W, filters.bottom()),
        pos2(inner.right(), inner.bottom()),
    );
    let right = if details {
        panel.left() - 1.0 - 24.0
    } else {
        b.right() - 24.0
    };
    let rest = Rect::from_min_max(
        pos2(b.left() + 24.0, filters.bottom()),
        pos2(right.max(b.left() + 24.0), inner.bottom()),
    );
    if details {
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(panel.left() - 1.0, panel.top()),
                pos2(panel.left(), panel.bottom()),
            ),
            0.0,
            t.border,
        );
        ui.scope_builder(UiBuilder::new().max_rect(panel.shrink(16.0)), |ui| {
            detail_panel(ui, t, home, acts)
        });
    }
    ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| {
        if home.folder_missing() {
            empty(
                ui,
                t,
                Ico::FolderSearch,
                tr(Msg::FolderMissing),
                None,
                Some(acts),
            );
        } else if !home.has_captures() {
            let hint = tf(Msg::HomeSubtitle, &[&area]);
            empty(
                ui,
                t,
                Ico::Images,
                tr(Msg::NoCapturesYet),
                Some(&hint),
                None,
            );
        } else {
            gallery(ui, t, home, shared.settings.home_view, acts);
        }
    });
}

/// The period pills on the left; search and the folder on the right.
fn filter_row(
    ui: &mut Ui,
    t: &Tokens,
    home: &mut Home,
    view: HomeView,
    details: bool,
    acts: &mut Vec<MainAction>,
) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let [all, today, week] = home.counts();
    for (p, msg, n) in [
        (Period::All, Msg::FilterAll, all),
        (Period::Today, Msg::Today, today),
        (Period::Week, Msg::ThisWeek, week),
    ] {
        if kit::filter_pill(ui, t, tr(msg), n, home.filter() == p).clicked() {
            acts.push(MainAction::Filter(p));
        }
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if kit::button(
            ui,
            t,
            BtnKind::Secondary,
            Some(Ico::FolderOpen),
            tr(Msg::OpenFolder),
            None,
        )
        .clicked()
        {
            acts.push(MainAction::OpenFolder);
        }
        if kit::icon_button(ui, t, Ico::PanelRight, details, tr(Msg::Details)).clicked() {
            acts.push(MainAction::SetHomeDetails(!details));
        }
        let mut v = view;
        let items = [
            (HomeView::Grid, Ico::Grid, tr(Msg::GridView)),
            (HomeView::List, Ico::List, tr(Msg::ListView)),
        ];
        if kit::icon_segmented(ui, t, &mut v, &items).changed() {
            acts.push(MainAction::SetHomeView(v));
        }
        let mut q = home.query().to_string();
        if kit::text_field(
            ui,
            t,
            &mut q,
            tr(Msg::SearchByName),
            Some(Ico::Search),
            240.0,
        )
        .changed()
        {
            acts.push(MainAction::Search(q));
        }
    });
}

/// Title and subtitle with the capture buttons on their right, or under them when the window is
/// too narrow for one row. The buttons' width is remembered from the frame before.
fn header(ui: &mut Ui, t: &Tokens, area: &str, full: &str, acts: &mut Vec<MainAction>) {
    // The design's 10 px between buttons and 4 px between title and subtitle.
    ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
    let title = tr(Msg::Home);
    let sub = tf(Msg::HomeSubtitle, &[area]);
    let sub = sub.as_str();
    let title_w = {
        let p = ui.painter();
        let w = |s: &str, f| p.layout_no_wrap(s.to_string(), f, t.text).size().x;
        w(title, font(22.0, Weight::Bold)).max(w(sub, font(13.0, Weight::Regular)))
    };
    let id = egui::Id::unique("takeui-home-buttons");
    let buttons_w = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(720.0);
    let titles = |ui: &mut Ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .font(font(22.0, Weight::Bold))
                    .color(t.text),
            );
            ui.label(
                RichText::new(sub)
                    .font(font(13.0, Weight::Regular))
                    .color(t.muted),
            );
        });
    };
    let w = if ui.available_width() >= title_w + buttons_w + 24.0 {
        ui.horizontal(|ui| {
            titles(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.scope(|ui| capture_buttons(ui, t, area, full, acts, true))
                    .response
                    .rect
                    .width()
            })
            .inner
        })
        .inner
    } else {
        titles(ui);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.scope(|ui| capture_buttons(ui, t, area, full, acts, false))
                .response
                .rect
                .width()
        })
        .inner
    };
    ui.ctx().data_mut(|d| d.insert_temp(id, w));
}

/// Full screen, in 3 seconds, open image and capture area (primary), left to right on screen;
/// `rtl` when the layout places them from the right.
fn capture_buttons(
    ui: &mut Ui,
    t: &Tokens,
    area: &str,
    full: &str,
    acts: &mut Vec<MainAction>,
    rtl: bool,
) {
    let mut items = [
        (
            BtnKind::Secondary,
            Ico::Monitor,
            Msg::FullScreen,
            Some(full),
            MainAction::Capture(Shot::Full),
        ),
        (
            BtnKind::Secondary,
            Ico::Timer,
            Msg::In3Seconds,
            None,
            MainAction::CaptureIn3s,
        ),
        (
            BtnKind::Secondary,
            Ico::ImagePlus,
            Msg::OpenImage,
            Some("Ctrl+O"),
            MainAction::OpenImage,
        ),
        (
            BtnKind::Primary,
            Ico::Scan,
            Msg::CaptureArea,
            Some(area),
            MainAction::Capture(Shot::Area),
        ),
    ];
    if rtl {
        items.reverse();
    }
    for (kind, icon, msg, keys, act) in items {
        if kit::button(ui, t, kind, Some(icon), tr(msg), keys).clicked() {
            acts.push(act);
        }
    }
}

/// The detail panel's width, its line included.
const PANEL_W: f32 = 268.0;
/// The column titles over the list.
const TITLES_H: f32 = 30.0;

/// One row of the gallery, by place among the shown captures: a row of cards from the first
/// one, or one capture's list row.
#[derive(Clone, Copy)]
enum Row {
    Cards(usize),
    Item(usize),
}

/// The gallery's rows for `shown` captures with where each starts and how tall it is, for `cols`
/// cards of `card_h` (or the list), and the height they take together.
fn layout(shown: usize, view: HomeView, cols: usize, card_h: f32) -> (Vec<(Row, f32, f32)>, f32) {
    let mut rows = Vec::new();
    let mut y = 0.0;
    match view {
        HomeView::Grid => {
            for start in (0..shown).step_by(cols.max(1)) {
                rows.push((Row::Cards(start), y, card_h));
                y += card_h + GAP;
            }
        }
        HomeView::List => {
            for n in 0..shown {
                rows.push((Row::Item(n), y, kit::ROW_H));
                y += kit::ROW_H + 2.0;
            }
        }
    }
    (rows, y)
}

/// The captures, newest first, as cards or list rows, each telling its day. Only the rows in view
/// are drawn, so only they ask for thumbnails.
fn gallery(ui: &mut Ui, t: &Tokens, home: &mut Home, view: HomeView, acts: &mut Vec<MainAction>) {
    let shown = home.shown().to_vec();
    let now = chrono::Local::now();
    let avail = ui.available_size();
    let room = avail.y
        - if view == HomeView::List {
            TITLES_H
        } else {
            0.0
        };
    // The scroll bar takes its room beside the rows, and only when they overflow.
    let height = |w: f32| {
        let (cols, card) = grid(w);
        layout(shown.len(), view, cols, card.y).1
    };
    let width = if height(avail.x) > room {
        avail.x - ui.spacing().scroll.allocated_width()
    } else {
        avail.x
    };
    let (cols, card) = grid(width);
    let (rows, total) = layout(shown.len(), view, cols, card.y);
    home.set_columns(if view == HomeView::Grid { cols } else { 1 });
    let reveal = home.take_reveal();
    let chosen = home.selected().map(|p| p.to_path_buf());
    let chosen_row = |row: Row| match (row, &chosen) {
        (Row::Item(n), Some(p)) => home.entry(shown[n]).path == *p,
        (Row::Cards(start), Some(p)) => shown
            .iter()
            .skip(start)
            .take(cols)
            .any(|&i| home.entry(i).path == *p),
        _ => false,
    };
    let reveal_at = if reveal {
        rows.iter()
            .find(|(row, _, _)| chosen_row(*row))
            .map(|&(_, y, h)| (y, h))
    } else {
        None
    };
    if view == HomeView::List {
        column_titles(ui, t, width);
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.max_rect().min;
            if let Some((y, h)) = reveal_at {
                ui.scroll_to_rect(
                    Rect::from_min_size(origin + vec2(0.0, y), vec2(width, h)),
                    None,
                );
            }
            for &(row, y, h) in &rows {
                if y + h < viewport.min.y || y > viewport.max.y {
                    continue;
                }
                let r = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, h));
                // Rows and cards go by their capture's path, so an open menu never passes to
                // another capture when the folder changes.
                let first = match row {
                    Row::Cards(n) | Row::Item(n) => home.entry(shown[n]).path.clone(),
                };
                ui.scope_builder(
                    UiBuilder::new().max_rect(r).id_salt(("gallery-row", first)),
                    |ui| match row {
                        Row::Cards(start) => {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = GAP;
                                for &i in shown.iter().skip(start).take(cols) {
                                    let path = home.entry(i).path.clone();
                                    ui.push_id(path, |ui| {
                                        card_item(ui, t, home, i, card, now, acts)
                                    });
                                }
                            });
                        }
                        Row::Item(n) => list_item(ui, t, home, shown[n], width, now, acts),
                    },
                );
            }
        });
}

/// The list's column titles, on the columns of its rows.
fn column_titles(ui: &mut Ui, t: &Tokens, width: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, TITLES_H), egui::Sense::hover());
    let c = kit::list_columns(width);
    let p = ui.painter();
    for (at, m) in [
        (Some(c.name), Msg::ColName),
        (c.cells[0], Msg::ColDate),
        (c.cells[1], Msg::ColDimensions),
        (c.cells[2], Msg::ColSize),
    ] {
        let Some(at) = at else {
            continue;
        };
        p.text(
            pos2(r.left() + at, r.center().y),
            egui::Align2::LEFT_CENTER,
            tr(m).to_uppercase(),
            font(11.0, Weight::Medium),
            t.faint,
        );
    }
    p.rect_filled(
        Rect::from_min_max(pos2(r.left(), r.bottom() - 1.0), r.max),
        0.0,
        t.line,
    );
}

/// "1920 × 1080 · 412 KB" once the thumbnail knows the size, else the file size.
fn meta_of(dims: Option<[u32; 2]>, size: &str) -> String {
    match dims {
        Some([w, h]) => tf(Msg::CaptureMeta, &[&w.to_string(), &h.to_string(), size]),
        None => size.to_string(),
    }
}

/// A capture's time of day ("14:32").
fn time_of(e: &crate::system::library::Entry) -> String {
    chrono::DateTime::<chrono::Local>::from(e.modified)
        .format("%H:%M")
        .to_string()
}

/// One capture as a card.
fn card_item(
    ui: &mut Ui,
    t: &Tokens,
    home: &mut Home,
    i: usize,
    card: egui::Vec2,
    now: chrono::DateTime<chrono::Local>,
    acts: &mut Vec<MainAction>,
) {
    let e = home.entry(i).clone();
    let (thumb, dims) = home.card_thumb(&e.path);
    let meta = meta_of(dims, &human_size(e.bytes));
    let line = format!("{} · {meta}", when_label(e.modified, now));
    let selected = home.selected() == Some(e.path.as_path());
    let r = kit::capture_card(ui, t, card, thumb, &e.name, &line, selected);
    item_answers(t, &e, &meta, r, acts);
}

/// One capture as a list row.
fn list_item(
    ui: &mut Ui,
    t: &Tokens,
    home: &mut Home,
    i: usize,
    width: f32,
    now: chrono::DateTime<chrono::Local>,
    acts: &mut Vec<MainAction>,
) {
    let e = home.entry(i).clone();
    let (thumb, dims) = home.card_thumb(&e.path);
    let dims_text = dims.map(|[w, h]| format!("{w} × {h}")).unwrap_or_default();
    let size = human_size(e.bytes);
    let selected = home.selected() == Some(e.path.as_path());
    let r = kit::capture_row(
        ui,
        t,
        width,
        thumb,
        &e.name,
        [&when_label(e.modified, now), &dims_text, &size],
        selected,
    );
    let meta = meta_of(dims, &size);
    item_answers(t, &e, &meta, r, acts);
}

/// What a card or a row asked for: its buttons, a double click (edit), a click (select), and the
/// same actions in its right-click menu.
fn item_answers(
    t: &Tokens,
    e: &crate::system::library::Entry,
    meta: &str,
    r: kit::CardResponse,
    acts: &mut Vec<MainAction>,
) {
    if let Some(a) = r.action {
        acts.push(match a {
            CardAction::Copy => MainAction::Copy(e.path.clone()),
            CardAction::Edit => MainAction::Edit(e.path.clone()),
            CardAction::Folder => MainAction::ShowInFolder(e.path.clone()),
            CardAction::Trash => MainAction::Trash(e.path.clone()),
        });
    } else if r.resp.double_clicked() {
        acts.push(MainAction::Edit(e.path.clone()));
    } else if r.resp.clicked() {
        acts.push(MainAction::Select(e.path.clone()));
    }
    let resp = r.resp.on_hover_text(details(e, meta));
    let p = || e.path.clone();
    let rows = [
        MenuRow::Item(Ico::Copy, tr(Msg::Copy), MainAction::Copy(p())),
        MenuRow::Item(Ico::Edit, tr(Msg::EditInEditor), MainAction::Edit(p())),
        MenuRow::Item(
            Ico::FolderOpen,
            tr(Msg::ShowInFolder),
            MainAction::ShowInFolder(p()),
        ),
        MenuRow::Item(Ico::Pin, tr(Msg::PinOnScreen), MainAction::Pin(p())),
        MenuRow::Line,
        MenuRow::Item(Ico::Trash, tr(Msg::Delete), MainAction::Trash(p())),
    ];
    if let Some(a) = kit::context_menu(&resp, t, &rows) {
        acts.push(a);
    }
}

/// The chosen capture's preview, name, day and time, dimensions, size and folder, then Copy,
/// Open in the editor, Show in folder, Pin and Delete; a hint when nothing is chosen.
fn detail_panel(ui: &mut Ui, t: &Tokens, home: &mut Home, acts: &mut Vec<MainAction>) {
    let area = ui.max_rect();
    let Some(e) = home.selected_entry() else {
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            tr(Msg::DetailsEmpty),
            font(12.0, Weight::Regular),
            t.faint,
        );
        return;
    };
    // The panel scrolls when the window is too short for it.
    egui::ScrollArea::vertical()
        .id_salt("detail-panel")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = ui.available_width();
            ui.spacing_mut().item_spacing.y = 12.0;
            let (thumb, dims) = home.card_thumb(&e.path);
            let (pic, _) =
                ui.allocate_exact_size(vec2(w, (w * 0.62).round()), egui::Sense::hover());
            kit::thumb(ui.painter(), t, pic, thumb);
            let now = chrono::Local::now();
            let day = crate::screens::home::day_label(
                crate::system::library::day_of(e.modified, now),
                now,
            );
            let line = |s: &str, size: f32, weight: Weight, c: egui::Color32| {
                let mut job =
                    egui::text::LayoutJob::simple_singleline(s.to_string(), font(size, weight), c);
                job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                job
            };
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(line(&e.name, 13.0, Weight::Bold, t.text));
                ui.label(line(
                    &tf(Msg::DayAtTime, &[&day, &time_of(&e)]),
                    12.0,
                    Weight::Regular,
                    t.muted,
                ));
            });
            let folder = e
                .path
                .parent()
                .map(|p| {
                    let parts: Vec<_> = p.components().rev().take(2).collect();
                    parts
                        .iter()
                        .rev()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(std::path::MAIN_SEPARATOR_STR)
                })
                .unwrap_or_default();
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let dims_text = dims.map(|[a, b]| format!("{a} × {b}")).unwrap_or_default();
                for (m, v) in [
                    (Msg::ColDimensions, dims_text.as_str()),
                    (Msg::ColSize, &human_size(e.bytes)),
                    (Msg::CapturesFolder, folder.as_str()),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(tr(m))
                                .font(font(12.0, Weight::Regular))
                                .color(t.muted),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(line(v, 12.0, Weight::Regular, t.text));
                        });
                    });
                }
            });
            let (rule, _) = ui.allocate_exact_size(vec2(w, 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rule, 0.0, t.border);
            let path = e.path.clone();
            let copy_keys = if cfg!(target_os = "macos") {
                "Cmd+C"
            } else {
                "Ctrl+C"
            };
            if kit::wide_button(
                ui,
                t,
                BtnKind::Primary,
                Some(Ico::Copy),
                tr(Msg::Copy),
                Some(copy_keys),
                w,
            )
            .clicked()
            {
                acts.push(MainAction::Copy(path.clone()));
            }
            if kit::wide_button(
                ui,
                t,
                BtnKind::Secondary,
                Some(Ico::Edit),
                tr(Msg::EditInEditor),
                Some("Enter"),
                w,
            )
            .clicked()
            {
                acts.push(MainAction::Edit(path.clone()));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if kit::wide_button(
                    ui,
                    t,
                    BtnKind::Ghost,
                    Some(Ico::FolderOpen),
                    tr(Msg::ShowInFolder),
                    None,
                    w - 32.0 - 8.0,
                )
                .clicked()
                {
                    acts.push(MainAction::ShowInFolder(path.clone()));
                }
                if kit::icon_button(ui, t, Ico::Pin, false, tr(Msg::PinOnScreen)).clicked() {
                    acts.push(MainAction::Pin(path.clone()));
                }
            });
            // Delete at the panel's foot, away from the others, when there is room below them; else
            // right after them.
            let mut delete = |ui: &mut Ui| {
                if kit::button(
                    ui,
                    t,
                    BtnKind::DangerGhost,
                    Some(Ico::Trash),
                    tr(Msg::Delete),
                    Some("Del"),
                )
                .clicked()
                {
                    acts.push(MainAction::Trash(path.clone()));
                }
            };
            let foot = Rect::from_min_size(pos2(area.left(), area.bottom() - 32.0), vec2(w, 32.0));
            if ui.min_rect().bottom() + ui.spacing().item_spacing.y <= foot.top() {
                ui.scope_builder(UiBuilder::new().max_rect(foot), delete);
            } else {
                delete(ui);
            }
        });
}

/// The card's tooltip: size, file size, folder and date.
fn details(e: &crate::system::library::Entry, meta: &str) -> String {
    let when = chrono::DateTime::<chrono::Local>::from(e.modified).format("%d/%m/%Y %H:%M");
    let folder = e
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    format!("{}\n{meta}\n{folder}\n{when}", e.name)
}

/// 412 KB, 1.2 MB: decimal units, as file managers show them.
fn human_size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{} KB", bytes.div_ceil(1000))
    }
}

/// An empty gallery: an icon, the reason, a hint, and "Choose folder" when the folder is missing.
fn empty(
    ui: &mut Ui,
    t: &Tokens,
    icon: Ico,
    title: &str,
    hint: Option<&str>,
    choose: Option<&mut Vec<MainAction>>,
) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(crate::ui::icon::glyph(icon).to_string())
                .font(crate::ui::icon::font(28.0))
                .color(t.faint),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .font(font(15.0, Weight::Medium))
                .color(t.text),
        );
        if let Some(h) = hint {
            ui.label(
                RichText::new(h)
                    .font(font(13.0, Weight::Regular))
                    .color(t.muted),
            );
        }
        if let Some(acts) = choose {
            ui.add_space(10.0);
            if kit::button(
                ui,
                t,
                BtnKind::Secondary,
                Some(Ico::FolderOpen),
                tr(Msg::ChooseFolder),
                None,
            )
            .clicked()
            {
                acts.push(MainAction::ChooseFolder);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::screens::home::{Home, MainAction};
    use crate::system::library::{Entry, Period};
    use crate::system::settings::HomeView;
    use crate::ui::i18n::{Msg, every, t as tr};
    use crate::ui::kit::Ico;
    use crate::views::testing::{
        click, content_box, double_click, menu_row_width, painted_at, right_click, run, run_painted,
    };
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, modified: SystemTime) -> Entry {
        Entry {
            path: PathBuf::from(name),
            name: name.to_string(),
            modified,
            bytes: 412_000,
        }
    }

    /// The box and the line under the Início header, in Portuguese with these keys.
    fn header_line(area: &str, full: &str) -> (egui::Rect, egui::Rect, Vec<(String, egui::Rect)>) {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        shared.settings.hotkey_area = area.into();
        shared.settings.hotkey_full = full.into();
        let (_, painted, blocks) =
            run_painted(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let t = shared.tokens(None);
        let b = content_box(&blocks, &t);
        let line = blocks
            .iter()
            .map(|(r, _)| *r)
            .find(|r| r.height() == 1.0 && r.width() >= b.width() - 2.0 && r.top() > b.top())
            .expect("a line across the box");
        (b, line, painted)
    }

    #[test]
    fn a_line_parts_the_header_from_the_filters() {
        // "Início · escuro" (Portuguese, PrtSc): the header is 87 px tall, then a line across the
        // whole box.
        let before = crate::ui::i18n::lang();
        crate::ui::i18n::set_lang(crate::ui::i18n::Lang::Pt);
        let (b, line, _) = header_line("PrintScreen", "shift+PrintScreen");
        crate::ui::i18n::set_lang(before);
        assert!(
            (line.top() - b.top() - 87.0).abs() <= 1.0,
            "{line:?} in {b:?}"
        );
    }

    #[test]
    fn a_wrapped_header_pushes_the_line_down() {
        // macOS's keys (Ctrl+Shift+2) in Portuguese do not fit beside the title at 1160 px: the
        // buttons go under it, and the line keeps 18 px below them.
        let before = crate::ui::i18n::lang();
        crate::ui::i18n::set_lang(crate::ui::i18n::Lang::Pt);
        let (b, line, painted) = header_line("ctrl+shift+Digit2", "ctrl+shift+Digit1");
        crate::ui::i18n::set_lang(before);
        let header_bottom = painted
            .iter()
            .filter(|(_, r)| r.top() > b.top() && r.bottom() < line.top())
            .map(|(_, r)| r.bottom())
            .fold(0.0, f32::max);
        assert!(line.top() > b.top() + 87.0, "the header wrapped: {line:?}");
        assert!(
            line.top() >= header_bottom + 17.0,
            "{line:?} under {header_bottom}"
        );
    }

    #[test]
    fn a_grid_without_a_scroll_bar_takes_the_full_width() {
        // One row fits at this size: the cards line up with the buttons above.
        let now = SystemTime::now();
        let few = (0..5).map(|i| entry(&format!("{i}.png"), now)).collect();
        let (mut home, mut shared) = Home::for_tests(few);
        // The grid alone, without the detail panel.
        shared.settings.home_details = false;
        let (_, _, blocks) =
            run_painted(&mut home, &mut shared, (1160.0, 740.0), true, vec![vec![]]);
        let t = shared.tokens(None);
        let b = content_box(&blocks, &t);
        let last = blocks
            .iter()
            .filter(|(r, c)| *c == t.input && r.height() > 60.0 && r.width() > 100.0)
            .map(|(r, _)| r.right())
            .fold(f32::MIN, f32::max);
        // 24 px from the box's edge, the picture 6 px inside the card.
        let edge = b.right() - 24.0 - 6.0;
        assert!(
            (last - edge).abs() <= 1.0,
            "picture edge {last}, expected {edge}"
        );
    }

    #[test]
    fn the_grid_leaves_the_scroll_bar_its_room() {
        let now = SystemTime::now();
        let many = (0..40).map(|i| entry(&format!("{i}.png"), now)).collect();
        let (mut home, mut shared) = Home::for_tests(many);
        let (_, _, blocks) =
            run_painted(&mut home, &mut shared, (1160.0, 740.0), true, vec![vec![]]);
        let t = shared.tokens(None);
        let b = content_box(&blocks, &t);
        // Cards too wide for the room would push the box past the window's 8 px margin.
        assert!((b.right() - (1160.0 - 8.0)).abs() <= 0.5, "{b:?}");
        let ctx = egui::Context::default();
        crate::ui::style::apply(&ctx, &t);
        let room = ctx.global_style().spacing.scroll.allocated_width();
        // A card's picture sits 6 px inside the card; the bar's track is narrow.
        let last = blocks
            .iter()
            .filter(|(r, c)| *c == t.input && r.height() > 60.0 && r.width() > 100.0)
            .map(|(r, _)| r.right())
            .fold(f32::MIN, f32::max);
        assert!(last > b.center().x, "the cards");
        assert!(
            last <= b.right() - 25.0 - room - 6.0 + 1.0,
            "picture edge {last}, box {b:?}, bar room {room}"
        );
    }

    #[test]
    fn a_double_click_opens_the_card_in_the_editor() {
        let now = SystemTime::now();
        let (mut home, mut shared) = Home::for_tests(vec![
            entry("a.png", now),
            entry("b.png", now - Duration::from_secs(60)),
        ]);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let b = painted_at(&painted, &["b.png"], |_| true).expect("b.png painted");
        let (acts, _) = run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            double_click(b),
        );
        assert_eq!(
            acts,
            vec![
                MainAction::Select(PathBuf::from("b.png")),
                MainAction::Edit(PathBuf::from("b.png"))
            ],
            "a double click selects, then opens, and nothing else"
        );
    }

    #[test]
    fn filters_narrow_the_grid_and_show_counts() {
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_577_836_800); // 2020-01-01
        let (mut home, mut shared) = Home::for_tests(vec![
            entry("today.png", SystemTime::now()),
            entry("old.png", old),
        ]);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        assert!(painted.iter().any(|(s, _)| s == "2"), "the All count");
        let today = painted_at(&painted, &every(Msg::Today), |r| r.top() > 40.0).unwrap();
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(today));
        assert!(
            acts.contains(&MainAction::Filter(Period::Today)),
            "{acts:?}"
        );
        for a in acts {
            home.apply(a, &egui::Context::default(), &mut shared);
        }
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let names: Vec<&str> = painted.iter().map(|(s, _)| s.as_str()).collect();
        assert!(names.contains(&"today.png"));
        assert!(!names.contains(&"old.png"), "filtered out");
    }

    #[test]
    fn the_header_fits_at_the_minimum_size() {
        // The capture buttons move under the title when the window is too narrow for one row.
        let now = SystemTime::now();
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", now)]);
        for size in [(900.0, 600.0), (1160.0, 740.0)] {
            let (_, painted) = run(&mut home, &mut shared, size, true, vec![]);
            let head: Vec<_> = painted
                .iter()
                .filter(|(_, r)| r.center().y > 40.0 && r.center().y < 190.0)
                .collect();
            for (i, (a, ra)) in head.iter().enumerate() {
                assert!(ra.right() <= size.0, "{a:?} {ra:?} outside at {size:?}");
                for (b, rb) in &head[i + 1..] {
                    assert!(
                        !ra.shrink(0.5).intersects(rb.shrink(0.5)),
                        "{a:?} {ra:?} overlaps {b:?} {rb:?} at {size:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_card_buttons_act_on_their_card() {
        use crate::ui::icon::{Ico, glyph};
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let card = painted_at(&painted, &["a.png"], |_| true).unwrap();
        let hover = vec![vec![egui::Event::PointerMoved(card)]];
        let (_, painted, blocks) =
            run_painted(&mut home, &mut shared, (1160.0, 740.0), true, hover.clone());
        // The card: the smallest tall rectangle around its name.
        let rect = blocks
            .iter()
            .map(|(r, _)| *r)
            .filter(|r| r.contains(card) && r.height() > 100.0 && r.width() < 400.0)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the card");
        let a = PathBuf::from("a.png");
        for (icon, want) in [
            (Ico::Copy, MainAction::Copy(a.clone())),
            (Ico::Edit, MainAction::Edit(a.clone())),
            (Ico::FolderOpen, MainAction::ShowInFolder(a.clone())),
            (Ico::Trash, MainAction::Trash(a.clone())),
        ] {
            let g = glyph(icon).to_string();
            // The card's own button: inside the card, above its name.
            let at = painted_at(&painted, &[g.as_str()], |r| {
                rect.contains(r.center()) && r.center().y < card.y
            })
            .expect("button painted");
            let mut frames = hover.clone();
            frames.extend(click(at));
            let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, frames);
            assert_eq!(acts, vec![want.clone()], "{icon:?}");
        }
    }

    #[test]
    fn paste_and_open_wait_while_typing_in_search() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let paste = egui::Event::Paste("x".into());
        let ctrl_o = egui::Event::Key {
            key: egui::Key::O,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let (acts, painted) = run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            vec![vec![paste.clone()], vec![ctrl_o.clone()]],
        );
        assert!(acts.contains(&MainAction::Paste), "{acts:?}");
        assert!(acts.contains(&MainAction::OpenImage), "{acts:?}");
        let search = painted_at(&painted, &every(Msg::SearchByName), |_| true).unwrap();
        let mut frames = click(search);
        frames.push(vec![paste]);
        frames.push(vec![ctrl_o]);
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, frames);
        assert!(!acts.contains(&MainAction::Paste), "{acts:?}");
        assert!(!acts.contains(&MainAction::OpenImage), "{acts:?}");
    }

    #[test]
    fn a_pasted_image_opens_unless_search_has_the_keyboard() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let image = egui::Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled(
            [2, 2],
            egui::Color32::RED,
        )));
        let (acts, painted) = run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            vec![vec![image.clone()]],
        );
        assert!(acts.contains(&MainAction::Paste), "{acts:?}");
        let search = painted_at(&painted, &every(Msg::SearchByName), |_| true).unwrap();
        let mut frames = click(search);
        frames.push(vec![image]);
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, frames);
        assert!(!acts.contains(&MainAction::Paste), "{acts:?}");
    }

    fn delete_key() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn trashed(acts: &[MainAction]) -> bool {
        acts.iter().any(|a| matches!(a, MainAction::Trash(_)))
    }

    #[test]
    fn delete_waits_while_typing_in_search() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        home.apply(
            MainAction::Select(PathBuf::from("a.png")),
            &egui::Context::default(),
            &mut shared,
        );
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let search = painted_at(&painted, &every(Msg::SearchByName), |_| true).unwrap();
        let mut frames = click(search);
        frames.push(vec![delete_key()]);
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, frames);
        assert!(
            !trashed(&acts),
            "Delete belongs to the search text: {acts:?}"
        );
    }

    #[test]
    fn gallery_keys_wait_behind_a_dialog() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let ctx = egui::Context::default();
        crate::system::fonts::FontLib::load().install(&ctx);
        home.apply(
            MainAction::Select(PathBuf::from("a.png")),
            &ctx,
            &mut shared,
        );
        let mut acts = Vec::new();
        for (i, events) in [vec![], vec![], vec![delete_key()]].into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1160.0, 740.0),
                )),
                time: Some(i as f64 * 0.05),
                events,
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                let t = shared.tokens(None);
                crate::views::begin_frame(ui.ctx(), &t);
                acts.extend(crate::views::main_window(ui, &t, &mut home, &mut shared));
                egui::Modal::new(egui::Id::unique("a-dialog")).show(ui.ctx(), |ui| {
                    ui.label("About");
                });
            });
            full.textures_delta.clear();
        }
        assert!(!trashed(&acts), "Delete belongs to the dialog: {acts:?}");
    }

    #[test]
    fn only_visible_rows_ask_for_thumbnails() {
        let now = SystemTime::now();
        for (view, most) in [(HomeView::Grid, 40), (HomeView::List, 20)] {
            let entries = (0..2000)
                .map(|i| entry(&format!("n{i}.png"), now - Duration::from_secs(i * 60)))
                .collect();
            let (mut home, mut shared) = Home::for_tests(entries);
            shared.settings.home_view = view;
            run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
            let asked = home.thumb_requests();
            assert!(asked > 0, "{view:?}: the visible items ask");
            assert!(
                asked <= most,
                "{view:?}: {asked} thumbnails asked for a 1160 × 740 window"
            );
        }
    }

    #[test]
    fn the_gallery_flows_without_day_titles() {
        let now = SystemTime::now();
        let day = Duration::from_secs(24 * 3600);
        let entries = vec![
            entry("a.png", now),
            entry("b.png", now - day),
            entry("c.png", now - day * 3),
        ];
        let (mut home, mut shared) = Home::for_tests(entries);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let y = |painted: &[(String, egui::Rect)], name: &str| {
            painted_at(painted, &[name], |_| true)
                .unwrap_or_else(|| panic!("{name}"))
                .y
        };
        assert_eq!(
            y(&painted, "a.png"),
            y(&painted, "c.png"),
            "one row of cards across the days"
        );
        let yesterday = format!("{} · ", tr(Msg::Yesterday));
        assert!(
            painted.iter().any(|(s, _)| s.starts_with(&yesterday)),
            "the card tells its day: {painted:?}"
        );
        shared.settings.home_view = HomeView::List;
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let (a, b, c) = (
            y(&painted, "a.png"),
            y(&painted, "b.png"),
            y(&painted, "c.png"),
        );
        assert!(
            ((b - a) - (c - b)).abs() < 0.5,
            "rows one after the other: {a} {b} {c}"
        );
        assert!(
            painted
                .iter()
                .any(|(s, r)| s == tr(Msg::Yesterday) && (r.center().y - b).abs() < 2.0),
            "the row tells its day"
        );
    }

    #[test]
    fn the_card_menu_is_as_wide_as_its_labels() {
        for view in [HomeView::Grid, HomeView::List] {
            let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
            shared.settings.home_view = view;
            shared.settings.home_details = false;
            let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
            let card = painted_at(&painted, &["a.png"], |_| true).expect("the capture");
            let (_, painted, blocks) = run_painted(
                &mut home,
                &mut shared,
                (1160.0, 740.0),
                true,
                right_click(card),
            );
            let item = painted_at(&painted, &[tr(Msg::EditInEditor)], |_| true)
                .unwrap_or_else(|| panic!("{view:?}: the menu in {painted:?}"));
            let w = menu_row_width(&blocks, item);
            assert!(w <= 240.0, "{view:?}: a {w} px menu in a 1160 px window");
        }
    }

    #[test]
    fn the_list_cuts_long_names() {
        let long = format!("{}.png", "captura-com-um-nome-muito-longo-".repeat(8));
        let (mut home, mut shared) = Home::for_tests(vec![entry(&long, SystemTime::now())]);
        shared.settings.home_view = HomeView::List;
        let (_, painted, blocks) =
            run_painted(&mut home, &mut shared, (900.0, 600.0), true, vec![]);
        let b = content_box(&blocks, &shared.tokens(None));
        let name = painted
            .iter()
            .find(|(s, _)| s.starts_with("captura-com"))
            .expect("the name");
        // Cut to its column (the whole name is over 1500 px).
        assert!(name.1.width() < 420.0, "{:?}", name.1);
        for (s, r) in &painted {
            assert!(
                r.right() <= b.right() + 0.5,
                "{s:?} at {r:?} runs past the box {b:?}"
            );
        }
    }

    #[test]
    fn the_list_row_buttons_act() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        shared.settings.home_view = HomeView::List;
        let ctx = egui::Context::default();
        home.apply(
            MainAction::Select(PathBuf::from("a.png")),
            &ctx,
            &mut shared,
        );
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        // The row, not the detail panel's copy of the name.
        let row = painted_at(&painted, &["a.png"], |r| r.right() < 1160.0 - 8.0 - 268.0)
            .expect("the row");
        let copy = painted_at(
            &painted,
            &[crate::ui::icon::glyph(Ico::Copy).to_string().as_str()],
            |r| (r.center().y - row.y).abs() < 12.0,
        )
        .expect("Copy on the chosen row");
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(copy));
        assert!(
            acts.contains(&MainAction::Copy(PathBuf::from("a.png"))),
            "{acts:?}"
        );
    }

    fn glyph_at(
        painted: &[(String, egui::Rect)],
        icon: Ico,
        inside: impl Fn(egui::Rect) -> bool,
    ) -> Option<egui::Pos2> {
        painted_at(
            painted,
            &[crate::ui::icon::glyph(icon).to_string().as_str()],
            inside,
        )
    }

    #[test]
    fn the_detail_panel_acts_on_the_selection() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let ctx = egui::Context::default();
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        assert!(
            painted_at(&painted, &every(Msg::DetailsEmpty), |_| true).is_some(),
            "nothing chosen yet"
        );
        home.apply(
            MainAction::Select(PathBuf::from("a.png")),
            &ctx,
            &mut shared,
        );
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let in_panel = |r: egui::Rect| r.left() > 1160.0 - 8.0 - 268.0;
        for (msg, act) in [
            (Msg::Copy, MainAction::Copy(PathBuf::from("a.png"))),
            (Msg::EditInEditor, MainAction::Edit(PathBuf::from("a.png"))),
            (
                Msg::ShowInFolder,
                MainAction::ShowInFolder(PathBuf::from("a.png")),
            ),
            (Msg::Delete, MainAction::Trash(PathBuf::from("a.png"))),
        ] {
            let at = painted_at(&painted, &every(msg), in_panel)
                .unwrap_or_else(|| panic!("{msg:?} in the panel"));
            let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(at));
            assert!(acts.contains(&act), "{msg:?}: {acts:?}");
        }
        let pin = glyph_at(&painted, Ico::Pin, in_panel).expect("Pin in the panel");
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(pin));
        assert!(
            acts.contains(&MainAction::Pin(PathBuf::from("a.png"))),
            "{acts:?}"
        );
        assert!(
            painted_at(&painted, &["a.png"], in_panel).is_some(),
            "the name in the panel"
        );
    }

    #[test]
    fn the_details_choice_is_kept() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let toggle = glyph_at(&painted, Ico::PanelRight, |_| true).expect("the Details toggle");
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(toggle));
        assert!(
            acts.contains(&MainAction::SetHomeDetails(false)),
            "{acts:?}"
        );
        shared.settings_dirty = false;
        home.apply(
            MainAction::SetHomeDetails(false),
            &egui::Context::default(),
            &mut shared,
        );
        assert!(!shared.settings.home_details && shared.settings_dirty);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        assert!(
            painted_at(&painted, &every(Msg::DetailsEmpty), |_| true).is_none(),
            "no panel"
        );
    }

    #[test]
    fn down_goes_to_the_card_below_across_days() {
        let now = SystemTime::now();
        let day = Duration::from_secs(24 * 3600);
        let entries: Vec<_> = (0..12u32)
            .map(|i| entry(&format!("{i:02}.png"), now - day * i))
            .collect();
        let (mut home, mut shared) = Home::for_tests(entries);
        shared.settings.home_details = false;
        home.apply(
            MainAction::Select(PathBuf::from("00.png")),
            &egui::Context::default(),
            &mut shared,
        );
        let down = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, painted) = run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            vec![vec![down]],
        );
        let below = home
            .selected()
            .expect("a choice")
            .to_string_lossy()
            .to_string();
        let at = |n: &str| painted_at(&painted, &[n], |_| true).unwrap_or_else(|| panic!("{n}"));
        let (a, b) = (at("00.png"), at(&below));
        assert!(
            (a.x - b.x).abs() < 1.0 && b.y > a.y,
            "{below} at {b:?} is right under 00.png at {a:?}"
        );
    }

    #[test]
    fn keys_move_and_act_on_the_chosen_capture() {
        let now = SystemTime::now();
        let entries = vec![
            entry("a.png", now),
            entry("b.png", now),
            entry("c.png", now),
        ];
        let (mut home, mut shared) = Home::for_tests(entries);
        shared.settings.home_view = HomeView::List;
        let ctx = egui::Context::default();
        home.apply(
            MainAction::Select(PathBuf::from("a.png")),
            &ctx,
            &mut shared,
        );
        let key = |k: egui::Key, m: egui::Modifiers| egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        };
        let none = egui::Modifiers::NONE;
        run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            vec![vec![key(egui::Key::ArrowDown, none)]],
        );
        assert_eq!(home.selected(), Some(std::path::Path::new("b.png")), "down");
        let ctrl_c = vec![
            egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
            egui::Event::Copy,
        ];
        for (events, act) in [
            (
                vec![key(egui::Key::Enter, none)],
                MainAction::Edit(PathBuf::from("b.png")),
            ),
            (ctrl_c, MainAction::Copy(PathBuf::from("b.png"))),
            (
                vec![key(egui::Key::Delete, none)],
                MainAction::Trash(PathBuf::from("b.png")),
            ),
        ] {
            let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![events]);
            assert!(acts.contains(&act), "{act:?}: {acts:?}");
        }
        run(
            &mut home,
            &mut shared,
            (1160.0, 740.0),
            true,
            vec![vec![key(egui::Key::ArrowUp, none)]],
        );
        assert_eq!(home.selected(), Some(std::path::Path::new("a.png")), "up");
    }

    #[test]
    fn the_home_fits_at_the_minimum_size_with_details() {
        use crate::ui::i18n::{Lang, set_lang};
        let long = format!("{}.png", "nome-de-captura-comprido-".repeat(6));
        for lang in [Lang::Pt, Lang::En] {
            set_lang(lang);
            for view in [HomeView::Grid, HomeView::List] {
                let (mut home, mut shared) = Home::for_tests(vec![entry(&long, SystemTime::now())]);
                shared.settings.home_view = view;
                home.apply(
                    MainAction::Select(PathBuf::from(&long)),
                    &egui::Context::default(),
                    &mut shared,
                );
                let (_, painted, blocks) =
                    run_painted(&mut home, &mut shared, (900.0, 600.0), true, vec![]);
                let b = content_box(&blocks, &shared.tokens(None));
                for (s, r) in &painted {
                    assert!(
                        r.right() <= b.right() + 0.5 && r.left() >= b.left() - 0.5,
                        "{lang:?} {view:?}: {s:?} at {r:?} outside {b:?}"
                    );
                }
                let panel_left = b.right() - 268.0;
                let gallery_texts: Vec<_> = painted
                    .iter()
                    .filter(|(_, r)| r.top() > 200.0 && r.right() < panel_left)
                    .collect();
                assert!(
                    !gallery_texts.is_empty(),
                    "{lang:?} {view:?}: the gallery beside the panel"
                );
                for (s, r) in &gallery_texts {
                    assert!(
                        r.right() <= panel_left,
                        "{lang:?} {view:?}: {s:?} runs into the panel"
                    );
                }
            }
        }
        set_lang(Lang::En);
    }

    #[test]
    fn the_detail_buttons_never_cover_each_other() {
        for size in [
            (900.0, 600.0),
            (1160.0, 600.0),
            (1160.0, 680.0),
            (1160.0, 740.0),
        ] {
            let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
            home.apply(
                MainAction::Select(PathBuf::from("a.png")),
                &egui::Context::default(),
                &mut shared,
            );
            let (_, painted) = run(&mut home, &mut shared, size, true, vec![]);
            let panel = size.0 - 8.0 - 268.0;
            let y = |m: Msg| {
                painted_at(&painted, &[tr(m)], |r| r.left() > panel)
                    .unwrap_or_else(|| panic!("{size:?}: {m:?} in the panel"))
                    .y
            };
            let ys = [
                y(Msg::Copy),
                y(Msg::EditInEditor),
                y(Msg::ShowInFolder),
                y(Msg::Delete),
            ];
            for pair in ys.windows(2) {
                assert!(
                    pair[1] - pair[0] >= 32.0,
                    "{size:?}: 32 px buttons one under the other: {ys:?}"
                );
            }
        }
    }

    #[test]
    fn the_view_choice_is_kept() {
        let (mut home, mut shared) = Home::for_tests(vec![entry("a.png", SystemTime::now())]);
        let (_, painted) = run(&mut home, &mut shared, (1160.0, 740.0), true, vec![]);
        let list = painted_at(
            &painted,
            &[crate::ui::icon::glyph(Ico::List).to_string().as_str()],
            |_| true,
        )
        .expect("the list toggle");
        let (acts, _) = run(&mut home, &mut shared, (1160.0, 740.0), true, click(list));
        assert!(
            acts.contains(&MainAction::SetHomeView(HomeView::List)),
            "{acts:?}"
        );
        shared.settings_dirty = false;
        home.apply(
            MainAction::SetHomeView(HomeView::List),
            &egui::Context::default(),
            &mut shared,
        );
        assert_eq!(shared.settings.home_view, HomeView::List);
        assert!(shared.settings_dirty);
    }
}
