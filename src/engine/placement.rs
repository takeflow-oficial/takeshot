//! Where floating bars go: the capture's bars beside the area and the editor's object bar over
//! the selection, kept on the screen or the canvas.

use egui::{Rect, Vec2, pos2};

/// Space between the object bar and the object.
pub const GAP: f32 = 10.0;

/// Space between the capture area and its main bar.
pub const CAPTURE_GAP: f32 = 12.0;

/// Space between the capture's two bars.
pub const BAR_SPACING: f32 = 6.0;

/// The capture's bars keep this far from the screen's edges.
pub const MARGIN: f32 = 8.0;

/// Where the capture's three parts go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureBars {
    /// The tools, in one column beside the area.
    pub tools: Rect,
    /// The tool's or the selected object's options, beside the tools.
    pub options: Option<Rect>,
    /// Pin, Save, Edit, Copy and close, in one row under (or over) the area.
    pub actions: Rect,
}

/// Places the capture's parts around `area` (the design's capture screens): the tools in a column to the
/// area's right (its left without room), from the area's top and down past its bottom only when
/// the column is taller than the area, in a place that never depends on the options; the options
/// beside the tools, else on the area's other side, from the same top; the
/// actions in a row under the area (over it without room) ending at its right edge, past its left
/// edge only when the row is wider than the area. Nothing leaves `screen` (`MARGIN` inside) or
/// covers another part; the area is covered only when no room is left outside it.
///
/// `avoid` is what the bars must not cover, the selected object with its handles: a part whose
/// place above covers it moves, and one that has no clear place stays where it was. The tools
/// take the opposite end of their edge (the area's top or the screen's bottom), then the screen's top, then
/// the other side, still by `avoid` alone; the options drop to `GAP` under it in their column, else rise to
/// `GAP` over it; the actions take, after their own place, right under it or right over it (ending
/// where it ends), then their other places, the first that is clear of it.
pub fn capture_bars(
    area: Rect,
    tools: Vec2,
    options: Option<Vec2>,
    actions: Vec2,
    screen: Rect,
    avoid: Option<Rect>,
) -> CaptureBars {
    let inside = screen.shrink(MARGIN);
    let right_room = inside.right() - area.right() - CAPTURE_GAP;
    let left_room = area.left() - CAPTURE_GAP - inside.left();
    #[derive(Clone, Copy, PartialEq)]
    enum Side {
        Right,
        Left,
    }
    // The tools' side depends on the tools alone, so they stay put while the options come and go:
    // the area's right, else its left, else (no room) over the area at the roomier screen edge.
    let (side, outside) = if right_room >= tools.x {
        (Side::Right, true)
    } else if left_room >= tools.x {
        (Side::Left, true)
    } else if right_room >= left_room {
        (Side::Right, false)
    } else {
        (Side::Left, false)
    };
    let fit_top = |top: f32, h: f32| top.min(inside.bottom() - h).max(inside.top());
    let tools_at = |side: Side, outside: bool, top: f32| {
        let x = match (side, outside) {
            (Side::Right, true) => area.right() + CAPTURE_GAP,
            (Side::Left, true) => area.left() - CAPTURE_GAP - tools.x,
            (Side::Right, false) => inside.right() - tools.x,
            (Side::Left, false) => inside.left(),
        };
        Rect::from_min_size(pos2(x, fit_top(top, tools.y)), tools)
    };
    let clear = |r: Rect| !avoid.is_some_and(|a| a.intersects(r));
    // Where they stand unless that covers `avoid`; else the same edge from the other end, then
    // the other side of the screen. Only `avoid` decides, so the options never move them.
    let other = match side {
        Side::Right => (Side::Left, left_room >= tools.x),
        Side::Left => (Side::Right, right_room >= tools.x),
    };
    let tops = [area.top(), inside.bottom() - tools.y, inside.top()];
    let (side, outside, tools_r) = [(side, outside), other]
        .into_iter()
        .flat_map(|(s, o)| tops.map(|top| (s, o, tools_at(s, o, top))))
        .find(|(_, _, r)| clear(*r))
        .unwrap_or((side, outside, tools_at(side, outside, area.top())));
    // The options: beside the tools away from the area, else on the area's other side (either
    // may use the screen's margin to keep off the area), else beside the tools over the area.
    let options_r = options.map(|o| {
        let (away, other) = match side {
            Side::Right => (
                screen.right() - tools_r.right() - BAR_SPACING,
                area.left() - CAPTURE_GAP - screen.left(),
            ),
            Side::Left => (
                tools_r.left() - BAR_SPACING - screen.left(),
                screen.right() - area.right() - CAPTURE_GAP,
            ),
        };
        let x = match side {
            Side::Right if outside && away >= o.x => tools_r.right() + BAR_SPACING,
            Side::Left if outside && away >= o.x => tools_r.left() - BAR_SPACING - o.x,
            Side::Right if outside && other >= o.x => area.left() - CAPTURE_GAP - o.x,
            Side::Left if outside && other >= o.x => area.right() + CAPTURE_GAP,
            Side::Right => tools_r.left() - BAR_SPACING - o.x,
            Side::Left => tools_r.right() + BAR_SPACING,
        };
        let r = Rect::from_min_size(pos2(x, fit_top(area.top(), o.y)), o);
        // Over `avoid`: down under it, else up over it, in the same column (so never over the tools).
        match avoid {
            Some(a) if a.intersects(r) => {
                let under = a.bottom() + GAP;
                let over = a.top() - GAP - o.y;
                if under + o.y <= inside.bottom() {
                    r.translate(egui::vec2(0.0, under - r.top()))
                } else if over >= inside.top() {
                    r.translate(egui::vec2(0.0, over - r.top()))
                } else {
                    r
                }
            }
            _ => r,
        }
    });
    // The actions: under, over, or inside the area's bottom.
    let below = area.bottom() + CAPTURE_GAP;
    let above = area.top() - CAPTURE_GAP - actions.y;
    let top = if below + actions.y <= inside.bottom() {
        below
    } else if above >= inside.top() {
        above
    } else {
        area.bottom() - CAPTURE_GAP - actions.y
    };
    let top = fit_top(top, actions.y);
    let right = area
        .right()
        .min(inside.right())
        .max(inside.left() + actions.x);
    let preferred = Rect::from_min_size(pos2(right - actions.x, top), actions);
    // Clear of the tools and options: else left of them, right of them, over or under them.
    let parts: Vec<Rect> = std::iter::once(tools_r).chain(options_r).collect();
    let column = parts.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
    let free = |r: &Rect| inside.contains_rect(*r) && !parts.iter().any(|p| p.intersects(*r));
    // Next to `avoid`, where the row is wanted when its place covers it: right under it, else
    // right over it, ending where it ends.
    let by_avoid = avoid.into_iter().flat_map(|a| {
        let right = a.right().min(inside.right()).max(inside.left() + actions.x);
        [a.bottom() + GAP, a.top() - GAP - actions.y]
            .map(|top| Rect::from_min_size(pos2(right - actions.x, top), actions))
    });
    let places: Vec<Rect> = std::iter::once(preferred)
        .chain(by_avoid)
        .chain([
            preferred.translate(egui::vec2(
                column.left() - BAR_SPACING - preferred.right(),
                0.0,
            )),
            preferred.translate(egui::vec2(
                column.right() + BAR_SPACING - preferred.left(),
                0.0,
            )),
            Rect::from_min_size(
                pos2(preferred.left(), column.top() - BAR_SPACING - actions.y),
                actions,
            ),
            Rect::from_min_size(
                pos2(preferred.left(), column.bottom() + BAR_SPACING),
                actions,
            ),
        ])
        .collect();
    // The first place that is also off `avoid`; with none, the first that is free.
    let actions_r = places
        .iter()
        .copied()
        .find(|r| free(r) && clear(*r))
        .or_else(|| places.iter().copied().find(free))
        .unwrap_or(preferred);
    CaptureBars {
        tools: tools_r,
        options: options_r,
        actions: actions_r,
    }
}

/// The object bar's rect, `size` big: centred over `object` and `GAP` above it; below it when the
/// canvas has no room above; at the canvas's top when neither fits. Always inside `canvas`, even
/// for an object scrolled off it.
pub fn object_bar(object: Rect, size: Vec2, canvas: Rect) -> Rect {
    let x = (object.center().x - size.x / 2.0)
        .min(canvas.right() - size.x)
        .max(canvas.left());
    let above = object.top() - GAP - size.y;
    let below = object.bottom() + GAP;
    let y = if above >= canvas.top() {
        above
    } else if below + size.y <= canvas.bottom() {
        below
    } else {
        canvas.top()
    };
    let y = y.min(canvas.bottom() - size.y).max(canvas.top());
    Rect::from_min_size(pos2(x, y), size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn capture_bars_stay_on_screen_and_apart() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        let inside = screen.shrink(MARGIN);
        let (tools, opts, actions) = (vec2(44.0, 450.0), vec2(220.0, 260.0), vec2(380.0, 42.0));
        let place = |area: Rect, with_opts: bool| {
            let b = capture_bars(
                area,
                tools,
                with_opts.then_some(opts),
                actions,
                screen,
                None,
            );
            let mut parts = vec![b.tools, b.actions];
            parts.extend(b.options);
            for (i, r) in parts.iter().enumerate() {
                // The options may touch the screen's edge when that keeps them off the area.
                let room = if Some(*r) == b.options {
                    screen
                } else {
                    inside
                };
                assert!(room.contains_rect(*r), "{r:?} off the screen for {area:?}");
                for o in &parts[i + 1..] {
                    assert!(!r.intersects(*o), "{r:?} over {o:?} for {area:?}");
                }
            }
            b
        };

        // Room all around: tools right of the area from its top, options beside them, actions
        // under it ending at its right edge; nothing over the area.
        let area = Rect::from_min_max(pos2(300.0, 100.0), pos2(1000.0, 650.0));
        let b = place(area, true);
        assert_eq!(b.tools.left(), area.right() + CAPTURE_GAP);
        assert_eq!(b.tools.top(), area.top());
        assert!(
            b.tools.bottom() <= area.bottom(),
            "the whole column within the area's height"
        );
        let o = b.options.unwrap();
        assert_eq!(o.left(), b.tools.right() + BAR_SPACING);
        assert_eq!(o.top(), area.top());
        assert_eq!(b.actions.top(), area.bottom() + CAPTURE_GAP);
        assert_eq!(b.actions.right(), area.right());
        for r in [b.tools, o, b.actions] {
            assert!(!r.intersects(area), "{r:?} over the area");
        }

        // At the right edge: tools and options go to the area's left.
        let area = Rect::from_min_max(pos2(1100.0, 100.0), pos2(1590.0, 600.0));
        let b = place(area, true);
        assert_eq!(b.tools.right(), area.left() - CAPTURE_GAP);
        assert_eq!(b.options.unwrap().right(), b.tools.left() - BAR_SPACING);

        // At the bottom: the actions go above the area, still ending at its right edge.
        let area = Rect::from_min_max(pos2(300.0, 400.0), pos2(1000.0, 880.0));
        let b = place(area, true);
        assert_eq!(b.actions.bottom(), area.top() - CAPTURE_GAP);
        assert_eq!(b.actions.right(), area.right());

        // An area lower than the tools: the column keeps one piece and runs past its bottom.
        let area = Rect::from_min_max(pos2(300.0, 100.0), pos2(1000.0, 300.0));
        let b = place(area, false);
        assert_eq!(b.tools.top(), area.top());
        assert_eq!(b.tools.size(), tools, "one column, never cut");
        assert!(b.tools.bottom() > area.bottom());

        // An area narrower than the actions: they run past its left edge, ending at its right.
        let area = Rect::from_min_max(pos2(700.0, 100.0), pos2(860.0, 600.0));
        let b = place(area, true);
        assert_eq!(b.actions.right(), area.right());
        assert!(b.actions.left() < area.left());

        // Room for the tools on each side but not for tools and options together: the tools
        // to the right, the options to the area's left, neither over the area.
        let area = Rect::from_min_max(pos2(240.0, 100.0), pos2(1352.0, 600.0));
        let b = place(area, true);
        assert_eq!(b.tools.left(), area.right() + CAPTURE_GAP);
        let o = b.options.unwrap();
        assert_eq!(o.right(), area.left() - CAPTURE_GAP);
        assert!(!o.intersects(area) && !b.tools.intersects(area));

        // The same with the options a pixel wider than the margin leaves: they touch the
        // screen's edge rather than cover the area.
        let screen_s = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
        let area = Rect::from_min_max(pos2(243.2, 122.4), pos2(1036.8, 554.4));
        let b = capture_bars(
            area,
            vec2(46.0, 451.0),
            Some(vec2(224.0, 171.0)),
            vec2(388.0, 42.0),
            screen_s,
            None,
        );
        let o = b.options.unwrap();
        assert!(!o.intersects(area) && !b.tools.intersects(area), "{b:?}");
        assert!(screen_s.contains_rect(o), "{o:?}");

        // A tiny area in a corner: everything on screen and apart.
        place(
            Rect::from_min_max(pos2(4.0, 860.0), pos2(40.0, 896.0)),
            true,
        );
        place(
            Rect::from_min_max(pos2(1560.0, 4.0), pos2(1596.0, 40.0)),
            true,
        );
    }

    /// The bars of a whole-screen area on a 1920 x 1080 screen (`Ctrl+A`), with the usual sizes.
    fn whole_screen(avoid: Option<Rect>, options: bool) -> (CaptureBars, Rect) {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let b = capture_bars(
            screen,
            vec2(46.0, 450.0),
            options.then_some(vec2(222.0, 240.0)),
            vec2(388.0, 42.0),
            screen,
            avoid,
        );
        (b, screen)
    }

    /// Every part on the screen (the options may touch its edge) and none over another.
    fn assert_apart(b: &CaptureBars, screen: Rect) {
        let mut parts = vec![b.tools, b.actions];
        parts.extend(b.options);
        for (i, r) in parts.iter().enumerate() {
            let room = if Some(*r) == b.options {
                screen
            } else {
                screen.shrink(MARGIN)
            };
            assert!(room.contains_rect(*r), "{r:?} off the screen: {b:?}");
            for o in &parts[i + 1..] {
                assert!(!r.intersects(*o), "{r:?} over {o:?}: {b:?}");
            }
        }
    }

    #[test]
    fn a_shape_in_the_corner_of_a_whole_screen_area_is_left_clear() {
        // A blur just drawn in the top right corner, where the tools and the options stand.
        let shape = Rect::from_min_max(pos2(1690.0, 16.0), pos2(1910.0, 206.0));
        let (before, screen) = whole_screen(None, true);
        assert!(
            before.tools.intersects(shape) && before.options.unwrap().intersects(shape),
            "the case: both over the shape, {before:?}"
        );
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        let o = b.options.unwrap();
        for r in [b.tools, o, b.actions] {
            assert!(!r.intersects(shape), "{r:?} over {shape:?}");
        }
        // The options drop under the shape in their own column.
        assert_eq!(o.top(), shape.bottom() + GAP);
        assert_eq!(o.left(), before.options.unwrap().left());
        // The tools keep their edge and size and take the opposite end of it.
        assert_eq!(b.tools.size(), before.tools.size());
        assert_eq!(b.tools.left(), before.tools.left());
        assert_eq!(b.tools.bottom(), screen.shrink(MARGIN).bottom());
    }

    #[test]
    fn only_the_part_over_the_shape_moves() {
        // Over the options (1638..1860 wide) and not over the tools (from 1866).
        let shape = Rect::from_min_max(pos2(1500.0, 16.0), pos2(1800.0, 206.0));
        let (before, screen) = whole_screen(None, true);
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        assert_eq!(b.tools, before.tools, "the tools were clear");
        assert_eq!(b.options.unwrap().top(), shape.bottom() + GAP);
        // Over the tools only.
        let shape = Rect::from_min_max(pos2(1880.0, 16.0), pos2(1910.0, 206.0));
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        assert_eq!(b.options, before.options, "the options were clear");
        assert!(!b.tools.intersects(shape));
    }

    #[test]
    fn the_options_rise_above_a_shape_with_no_room_under_it() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let bars = |area: Rect, shape: Rect| {
            let b = capture_bars(
                area,
                vec2(46.0, 450.0),
                Some(vec2(222.0, 240.0)),
                vec2(388.0, 42.0),
                screen,
                Some(shape),
            );
            assert_apart(&b, screen);
            b
        };
        // A shape low on the screen: the options, down there too, go over it.
        let area = Rect::from_min_max(pos2(0.0, 800.0), pos2(1920.0, 1080.0));
        let shape = Rect::from_min_max(pos2(1650.0, 850.0), pos2(1850.0, 1000.0));
        let o = bars(area, shape).options.unwrap();
        assert_eq!(o.bottom(), shape.top() - GAP);
        assert!(!o.intersects(shape));
        // A shape from top to bottom has no room over or under it: the options stay where they were.
        let shape = Rect::from_min_max(pos2(1650.0, 20.0), pos2(1850.0, 1060.0));
        let (without, _) = whole_screen(None, true);
        assert_eq!(bars(screen, shape).options, without.options);
    }

    #[test]
    fn the_tools_take_the_other_side_when_a_shape_fills_their_edge() {
        let (without, screen) = whole_screen(None, true);
        // The right edge is covered from top to bottom.
        let shape = Rect::from_min_max(pos2(1800.0, 8.0), pos2(1912.0, 1072.0));
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        assert_eq!(b.tools.size(), without.tools.size());
        assert_eq!(b.tools.left(), screen.shrink(MARGIN).left());
        assert!(!b.tools.intersects(shape));
        // Nothing is clear: everything stays where it was.
        let (b, _) = whole_screen(Some(screen), true);
        assert_eq!(b, without);
    }

    #[test]
    fn the_actions_sit_over_a_shape_at_the_bottom_of_the_screen() {
        // Low in the right corner, where the actions end: right over it, ending where it does,
        // not in the middle of the screen under the tools.
        let shape = Rect::from_min_max(pos2(1700.0, 960.0), pos2(1910.0, 1070.0));
        let (before, screen) = whole_screen(None, true);
        assert!(before.actions.intersects(shape), "the case: {before:?}");
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        assert_eq!(b.actions.bottom(), shape.top() - GAP, "{b:?}");
        assert_eq!(b.actions.right(), shape.right());
        assert_eq!(b.tools, before.tools);
        assert_eq!(b.options, before.options);
        // A wide shape across the bottom: the row still sits over it, at its right end.
        let shape = Rect::from_min_max(pos2(140.0, 935.0), pos2(1650.0, 1055.0));
        let (b, _) = whole_screen(Some(shape), true);
        assert_apart(&b, screen);
        assert_eq!(b.actions.bottom(), shape.top() - GAP, "{b:?}");
        assert_eq!(b.actions.right(), shape.right());
    }

    #[test]
    fn the_actions_sit_under_a_shape_when_there_is_room() {
        // The area ends well above the screen's bottom: the row is under the area, and a shape
        // that reaches into it has room under it.
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 700.0));
        let place = |avoid| {
            let b = capture_bars(
                area,
                vec2(46.0, 450.0),
                None,
                vec2(388.0, 42.0),
                screen,
                avoid,
            );
            assert_apart(&b, screen);
            b
        };
        let shape = Rect::from_min_max(pos2(1500.0, 690.0), pos2(1700.0, 770.0));
        assert!(place(None).actions.intersects(shape), "the case");
        let b = place(Some(shape));
        assert_eq!(b.actions.top(), shape.bottom() + GAP, "{b:?}");
        assert_eq!(b.actions.right(), shape.right());
    }

    #[test]
    fn a_shape_inside_an_area_with_room_around_changes_nothing() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        let area = Rect::from_min_max(pos2(300.0, 100.0), pos2(1000.0, 650.0));
        let place = |avoid| {
            capture_bars(
                area,
                vec2(44.0, 450.0),
                Some(vec2(220.0, 260.0)),
                vec2(380.0, 42.0),
                screen,
                avoid,
            )
        };
        let shape = Rect::from_min_max(pos2(700.0, 120.0), pos2(990.0, 300.0));
        assert_eq!(place(Some(shape)), place(None));
    }

    #[test]
    fn the_tools_stay_put_when_the_options_appear() {
        let (tools, opts, actions) = (vec2(44.0, 450.0), vec2(226.0, 260.0), vec2(380.0, 42.0));
        let check = |area: Rect, screen: Rect| {
            // With or without a shape to keep off: the tools never answer to the options.
            let corner =
                Rect::from_min_size(pos2(screen.right() - 260.0, 20.0), vec2(240.0, 160.0));
            for avoid in [None, Some(corner), Some(area.shrink(2.0))] {
                let without = capture_bars(area, tools, None, actions, screen, avoid).tools;
                let with = capture_bars(area, tools, Some(opts), actions, screen, avoid).tools;
                assert_eq!(with, without, "the tools moved for {area:?} on {screen:?}");
            }
        };
        // The case that showed it: 6 px short of room for both on the right.
        let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        check(
            Rect::from_min_max(pos2(100.0, 200.0), pos2(1310.0, 600.0)),
            big,
        );
        for size in [
            vec2(1280.0, 720.0),
            vec2(1600.0, 900.0),
            vec2(1920.0, 1080.0),
        ] {
            let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
            for left in (0..size.x as i32 - 40).step_by(37) {
                for right in (left + 40..=size.x as i32).step_by(53) {
                    for (top, bottom) in [(100.0, 650.0), (20.0, size.y - 20.0), (300.0, 400.0)] {
                        let area =
                            Rect::from_min_max(pos2(left as f32, top), pos2(right as f32, bottom));
                        check(area, screen);
                    }
                }
            }
        }
    }

    #[test]
    fn the_object_bar_sits_above_or_below_inside_the_canvas() {
        let canvas = Rect::from_min_size(pos2(0.0, 100.0), vec2(900.0, 600.0));
        let size = vec2(126.0, 36.0);
        let bar = |o: Rect| object_bar(o, size, canvas);

        // Room above: centred over the object, GAP above it.
        let o = Rect::from_min_max(pos2(300.0, 400.0), pos2(500.0, 500.0));
        let b = bar(o);
        assert_eq!(b.size(), size);
        assert_eq!(b.center().x, 400.0);
        assert_eq!(b.bottom(), o.top() - GAP);

        // An object at the top of the canvas: the bar goes below it.
        let o = Rect::from_min_max(pos2(300.0, 110.0), pos2(500.0, 200.0));
        assert_eq!(bar(o).top(), o.bottom() + GAP);

        // Near the right or left edge: pushed against it.
        let o = Rect::from_min_max(pos2(860.0, 400.0), pos2(895.0, 450.0));
        assert_eq!(bar(o).right(), canvas.right());
        let o = Rect::from_min_max(pos2(2.0, 400.0), pos2(30.0, 450.0));
        assert_eq!(bar(o).left(), canvas.left());

        // An object scrolled past the canvas's bottom or top: the bar stays inside.
        for o in [
            Rect::from_min_max(pos2(300.0, 800.0), pos2(500.0, 850.0)),
            Rect::from_min_max(pos2(300.0, -200.0), pos2(500.0, 20.0)),
        ] {
            assert!(canvas.contains_rect(bar(o)), "{o:?}: {:?}", bar(o));
        }

        // An object filling the canvas: still inside it.
        for o in [canvas, canvas.expand(50.0)] {
            assert!(canvas.contains_rect(bar(o)), "{:?}", bar(o));
        }
    }
}
