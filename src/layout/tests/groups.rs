//! Split groups (docs/wiki/Development:-Split-Groups.md).
//!
//! The test output is 1280 x 720 with 10 px gaps: a half-width column is 625 x 700 (tall, so
//! merges stack), a full-width one 1260 x 700 (wide, so merges go side by side). A merge of two
//! columns (`split-window`) spans both: two quarter-width columns make a half-width group.

use std::collections::HashMap;

use niri_config::animations::{Curve, EasingParams, Kind};

use super::*;

const GAPS: f64 = 10.;

fn options(default_width: f64) -> Options {
    Options {
        layout: niri_config::Layout {
            gaps: GAPS,
            default_column_width: Some(PresetSize::Proportion(default_width)),
            ..Default::default()
        },
        animations: niri_config::Animations {
            off: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn window(id: usize) -> Op {
    Op::AddWindow {
        params: TestWindowParams::new(id),
    }
}

/// Lets every window commit its requested size and finishes the animations.
fn settle() -> Vec<Op> {
    let mut ops: Vec<Op> = (0..=9).map(Op::Communicate).collect();
    ops.push(Op::CompleteAnimations);
    ops
}

thread_local! {
    static WIDTH: Cell<f64> = const { Cell::new(0.5) };
}

/// New windows open at the default column width (which niri.rs resolves before adding them).
fn with_widths(ops: impl IntoIterator<Item = Op>) -> Vec<Op> {
    let width = WIDTH.with(Cell::get);
    let mut all = Vec::new();
    for op in ops {
        let is_add = matches!(op, Op::AddWindow { .. });
        all.push(op);
        if is_add {
            all.push(Op::SetColumnWidth(SizeChange::SetProportion(width * 100.)));
        }
    }
    all
}

fn run(width: f64, ops: impl IntoIterator<Item = Op>) -> Layout<TestWindow> {
    WIDTH.with(|w| w.set(width));
    let mut all: Vec<Op> = vec![Op::AddOutput(1)];
    all.extend(with_widths(ops));
    all.extend(settle());
    check_ops_with_options(options(width), all)
}

fn more(layout: &mut Layout<TestWindow>, ops: impl IntoIterator<Item = Op>) {
    let mut all: Vec<Op> = with_widths(ops);
    all.extend(settle());
    check_ops_on_layout(layout, all);
}

/// Tile rectangles of the active workspace by window id, in output coordinates.
fn rects(layout: &Layout<TestWindow>) -> HashMap<usize, Rectangle<f64, Logical>> {
    layout
        .active_workspace()
        .unwrap()
        .tiles_with_render_positions()
        .filter(|(_, _, visible)| *visible)
        .map(|(tile, pos, _)| (tile.window().0.id, Rectangle::new(pos, tile.tile_size())))
        .collect()
}

/// The active workspace's columns: plain columns as `{1 2}` (stacked windows), groups as
/// `[...]` (side by side) and `(...)` (stacked) with window ids in their slots.
fn shape(layout: &Layout<TestWindow>) -> String {
    fn node(n: &group::Node, ids: &mut impl Iterator<Item = usize>, out: &mut String) {
        match n {
            group::Node::Slot { vacant: true } => out.push('_'),
            group::Node::Slot { vacant: false } => out.push_str(&ids.next().unwrap().to_string()),
            group::Node::Split { kind, children } => {
                let (open, close) = match kind {
                    group::SplitKind::SideBySide => ('[', ']'),
                    group::SplitKind::Stacked => ('(', ')'),
                };
                out.push(open);
                for (i, c) in children.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    node(&c.node, ids, out);
                }
                out.push(close);
            }
        }
    }

    let ws = layout.active_workspace().unwrap();
    let mut cols = Vec::new();
    for col in ws.scrolling().columns() {
        let mut ids = col.tiles().map(|(tile, _)| tile.window().0.id);
        let mut s = String::new();
        match col.group() {
            Some(g) => node(g.root(), &mut ids, &mut s),
            None => {
                let ids: Vec<_> = ids.map(|id| id.to_string()).collect();
                s = format!("{{{}}}", ids.join(" "));
            }
        }
        cols.push(s);
    }
    cols.join(" ")
}

fn focused(layout: &Layout<TestWindow>) -> usize {
    layout.focus().unwrap().0.id
}

fn group_id(layout: &Layout<TestWindow>, id: usize) -> Option<u64> {
    layout
        .workspaces()
        .find(|(_, _, ws)| ws.has_window(&id))
        .and_then(|(_, _, ws)| ws.scrolling().group_id_of(&id))
}

fn right(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.x + r.size.w
}

fn bottom(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.y + r.size.h
}

fn center(r: Rectangle<f64, Logical>) -> (f64, f64) {
    (r.loc.x + r.size.w / 2., r.loc.y + r.size.h / 2.)
}

/// Drags `id` from its centre to (x, y) and drops it there.
fn drag(layout: &Layout<TestWindow>, id: usize, x: f64, y: f64) -> Vec<Op> {
    let (cx, cy) = center(rects(layout)[&id]);
    vec![
        Op::InteractiveMoveBegin {
            window: id,
            output_idx: 1,
            px: cx,
            py: cy,
        },
        Op::InteractiveMoveUpdate {
            window: id,
            dx: 300.,
            dy: 300.,
            output_idx: 1,
            px: x,
            py: y,
        },
        Op::InteractiveMoveUpdate {
            window: id,
            dx: 0.,
            dy: 0.,
            output_idx: 1,
            px: x,
            py: y,
        },
        Op::InteractiveMoveEnd { window: id },
    ]
}

// -- Merging and rotating ---------------------------------------------------------------------

#[test]
fn split_merges_with_right_neighbour_stacked_in_a_tall_column() {
    let layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "(1 2)");
    assert_eq!(focused(&layout), 1);

    let r = rects(&layout);
    // Two quarter-width columns make a half-width group (both columns and the gap between
    // them), taller than wide: the window takes the top half.
    assert_eq!(r[&1].size, Size::from((625., 345.)));
    assert_eq!(r[&2].size, Size::from((625., 345.)));
    assert_eq!(r[&1].loc.x, r[&2].loc.x);
    assert_eq!(r[&2].loc.y - bottom(r[&1]), GAPS);
}

#[test]
fn split_merges_side_by_side_in_a_wide_column() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((625., 700.)));
    assert_eq!(r[&2].size, Size::from((625., 700.)));
    assert_eq!(r[&2].loc.x - right(r[&1]), GAPS);
}

#[test]
fn split_at_the_end_merges_left() {
    let layout = run(1., [window(1), window(2), Op::SplitWindow { id: None }]);
    // 2 is last: it merges into 1's column, on the side it came from.
    assert_eq!(shape(&layout), "[1 2]");
    assert_eq!(focused(&layout), 2);
}

/// Two columns of fixed widths (`SetFixed` window widths) with the first focused.
fn fixed_pair(first: i32, second: i32) -> Vec<Op> {
    vec![
        window(1),
        Op::SetColumnWidth(SizeChange::SetFixed(first)),
        window(2),
        Op::SetColumnWidth(SizeChange::SetFixed(second)),
        Op::FocusColumnFirst,
    ]
}

#[test]
fn split_merge_takes_both_columns_space_side_by_side() {
    // Mike's 1433 + 1097 on 2560 px, scaled to the test output: the group spans both columns
    // and the gap between them, so side-by-side windows stay side by side.
    let mut layout = run(0.5, fixed_pair(600, 400));
    let before = rects(&layout);
    assert_eq!(before[&1].size.w, 600.);
    assert_eq!(before[&2].size.w, 400.);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2]");
    assert_eq!(focused(&layout), 1);
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((500., 700.)));
    assert_eq!(r[&2].size, Size::from((500., 700.)));
    assert_eq!(r[&2].loc.x - right(r[&1]), GAPS);
    // Where both columns were, the source on its own side.
    assert_eq!(r[&1].loc.x, before[&1].loc.x);
    assert_eq!(right(r[&2]), right(before[&2]));
}

#[test]
fn split_merge_of_proportions_adds_them_up() {
    let layout = run(
        0.5,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    // ½ + ½: the whole view, wide, so side by side.
    assert_eq!(shape(&layout), "[1 2]");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((625., 700.)));
    assert_eq!(r[&2].size, Size::from((625., 700.)));
    assert_eq!(r[&2].loc.x - right(r[&1]), GAPS);
}

#[test]
fn split_merge_of_narrow_columns_stacks_in_their_combined_width() {
    let layout = run(
        0.5,
        [fixed_pair(300, 290), vec![Op::SplitWindow { id: None }]].concat(),
    );
    // 300 + 10 + 290 = 600 wide, 700 tall: stacked, but as wide as both columns.
    assert_eq!(shape(&layout), "(1 2)");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((600., 345.)));
    assert_eq!(r[&2].size, Size::from((600., 345.)));
}

#[test]
fn split_merge_of_full_width_columns_caps_at_the_view() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((625., 700.)));
    assert_eq!(r[&2].size, Size::from((625., 700.)));

    // Toggled full width counts as the whole view too, and fixed widths past it cap at it.
    let layout = run(
        0.5,
        [
            window(1),
            Op::MaximizeColumn,
            window(2),
            Op::SetColumnWidth(SizeChange::SetFixed(900)),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((625., 700.)));
    assert_eq!(r[&2].size, Size::from((625., 700.)));
}

#[test]
fn split_merge_at_the_end_mirrors() {
    // 2 is last: it merges left into 1's column and stays on the right.
    let mut layout = run(0.5, fixed_pair(400, 600));
    more(&mut layout, [Op::FocusColumnLast]);
    let before = rects(&layout);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2]");
    assert_eq!(focused(&layout), 2);
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((500., 700.)));
    assert_eq!(r[&2].size, Size::from((500., 700.)));
    assert_eq!(r[&1].loc.x, before[&1].loc.x);
    assert_eq!(right(r[&2]), right(before[&2]));
}

#[test]
fn split_merge_keeps_the_columns_after_in_place() {
    let mut layout = run(
        0.5,
        [
            window(1),
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            window(2),
            Op::SetColumnWidth(SizeChange::SetFixed(200)),
            window(3),
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::FocusColumnFirst,
        ],
    );
    let before = rects(&layout);
    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "(1 2) {3}");
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 510.);
    assert_eq!(r[&3], before[&3]);
}

#[test]
fn split_alone_does_nothing() {
    let layout = run(1., [window(1), Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "{1}");
}

/// Window ids on screen, by their rectangles.
fn on_screen(layout: &Layout<TestWindow>) -> Vec<usize> {
    let mut ids: Vec<usize> = rects(layout)
        .into_iter()
        .filter(|(_, r)| right(*r) > 1. && r.loc.x < 1279.)
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids
}

/// `[1 2]` (a full-width group of two half-width columns) and full-width 3 after it, focused.
fn group_then_full_width_window() -> Vec<Op> {
    vec![
        window(1),
        window(2),
        Op::FocusColumnFirst,
        Op::SplitWindow { id: None },
        window(3),
        Op::SetColumnWidth(SizeChange::SetProportion(100.)),
    ]
}

#[test]
fn split_of_the_only_window_on_screen_leaves_an_offscreen_group_alone() {
    // The report: "it splits three windows when only one window is visible". Mod+S on 3 used to
    // merge it into the group of two on its left, off screen: a group of three.
    let mut layout = run(0.5, group_then_full_width_window());
    assert_eq!(shape(&layout), "[1 2] {3}");
    assert_eq!(on_screen(&layout), [3]);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2] {3}");
    assert_eq!(focused(&layout), 3);
}

#[test]
fn split_of_the_only_window_on_screen_pairs_with_a_lone_neighbour_over_a_group() {
    // {4} {3} [1 2], full width, 3 focused and alone on screen: the group on its right is
    // skipped for the lone window on its left.
    let mut layout = run(1., [window(4), window(3), window(1), window(2)]);
    more(
        &mut layout,
        [
            Op::FocusWindow(1),
            Op::SplitWindow { id: None },
            Op::FocusWindow(3),
        ],
    );
    assert_eq!(shape(&layout), "{4} {3} [1 2]");
    assert_eq!(on_screen(&layout), [3]);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[4 3] [1 2]");
    assert_eq!(focused(&layout), 3);
}

#[test]
fn split_of_the_only_window_on_screen_leaves_an_offscreen_stack_alone() {
    // {1} {2 3}: 1 full width; the stack would turn into a group of three.
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::ConsumeOrExpelWindowLeft { id: None },
            Op::FocusColumnFirst,
        ],
    );
    assert_eq!(shape(&layout), "{1} {2 3}");
    assert_eq!(on_screen(&layout), [1]);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "{1} {2 3}");
}

#[test]
fn split_of_the_only_window_on_screen_still_pairs_with_a_lone_neighbour() {
    // Two full-width windows: the right one is off screen, but it's one window, so they pair.
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
}

#[test]
fn split_prefers_the_neighbour_on_screen() {
    // {1} {2} {3}, half width each, the view on 1 and 2: Mod+S on 2 merges with 1 (on screen),
    // not with 3 (off screen), though 3 is on the right.
    let mut layout = run(0.5, [window(1), window(2), window(3)]);
    // One at a time: each focus moves the view by the windows' committed sizes.
    more(&mut layout, [Op::FocusColumnFirst]);
    more(&mut layout, [Op::FocusColumnRight]);
    assert_eq!(on_screen(&layout), [1, 2]);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2] {3}");
    assert_eq!(focused(&layout), 2);
}

#[test]
fn split_next_to_a_group_on_screen_merges_as_a_third() {
    // Half-width 3 after the full-width group [1 2]: the group's right half (2) is on screen, so
    // Mod+S merges 3 into it, as an equal sibling.
    let mut layout = run(
        0.5,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            window(3),
        ],
    );
    assert_eq!(on_screen(&layout), [2, 3]);

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2 3]");
}

#[test]
fn split_skips_a_neighbour_that_cant_take_a_window() {
    // {4} {3} {1 2}, the last one tabbed: Mod+S on 3 skips it (on screen, on the right) for 4.
    let mut layout = run(
        0.5,
        [
            window(4),
            window(3),
            window(1),
            window(2),
            Op::ConsumeOrExpelWindowLeft { id: None },
            Op::ToggleColumnTabbedDisplay,
            Op::FocusColumnLeft,
        ],
    );
    assert_eq!(shape(&layout), "{4} {3} {1 2}");
    assert_eq!(focused(&layout), 3);
    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[4 3] {1 2}");
}

#[test]
fn split_again_rotates_and_back() {
    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let before = rects(&layout);
    assert_eq!(shape(&layout), "(1 2)");

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2]");
    let r = rects(&layout);
    assert_eq!(r[&1].size, Size::from((308., 700.)));
    assert_eq!(r[&2].size, Size::from((307., 700.)));

    more(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "(1 2)");
    assert_eq!(rects(&layout), before);
}

#[test]
fn three_side_by_side_are_equal() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            // 3 is at the end of the strip: it merges into the group on its left.
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2 3]");
    let r = rects(&layout);
    let widths: Vec<f64> = [1, 2, 3].iter().map(|id| r[id].size.w).collect();
    // ⅓ ⅓ ⅓, not ½ ¼ ¼.
    assert_eq!(widths, vec![414., 413., 413.]);
    assert_eq!(r[&2].loc.x - right(r[&1]), GAPS);
    assert_eq!(r[&3].loc.x - right(r[&2]), GAPS);
}

#[test]
fn rotating_nested_split_keeps_nesting() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusWindow(2),
        ],
    );
    // Put 3 below 2.
    more(
        &mut layout,
        [Op::SplitWindowBeside {
            id: Some(3),
            target: 2,
            side: group::Side::Bottom,
        }],
    );
    assert_eq!(shape(&layout), "[1 (2 3)]");
    more(&mut layout, [Op::SplitWindow { id: Some(3) }]);
    assert_eq!(shape(&layout), "[1 [2 3]]");
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 625.);
    assert_eq!(r[&2].size.w + r[&3].size.w + GAPS, 625.);
    more(&mut layout, [Op::SplitWindow { id: Some(3) }]);
    assert_eq!(shape(&layout), "[1 (2 3)]");
}

#[test]
fn split_on_plain_stack_puts_it_side_by_side() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::ConsumeOrExpelWindowLeft { id: None },
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
}

#[test]
fn tabbed_columns_are_left_alone() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::ConsumeOrExpelWindowLeft { id: None },
            Op::ToggleColumnTabbedDisplay,
            Op::SplitWindow { id: None },
            window(3),
            Op::SplitWindow { id: None },
        ],
    );
    // Neither the tabbed column nor a merge into it.
    assert_eq!(shape(&layout), "{1 2} {3}");
}

// -- Leaving and dissolving -------------------------------------------------------------------

#[test]
fn unsplit_pops_out_and_dissolves() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2 3]");

    // The middle one goes to the right of the group.
    more(&mut layout, [Op::UnsplitWindow { id: Some(2) }]);
    assert_eq!(shape(&layout), "[1 3] {2}");
    // It keeps its width.
    assert_eq!(rects(&layout)[&2].size.w, 413.);
    assert_eq!(focused(&layout), 3);

    // The leftmost one goes to the left; the group of one dissolves.
    more(&mut layout, [Op::UnsplitWindow { id: Some(1) }]);
    assert_eq!(shape(&layout), "{1} {3} {2}");
    assert_eq!(rects(&layout)[&3].size.w, 1260.);
}

#[test]
fn closing_a_window_gives_its_space_to_the_others() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
        ],
    );
    more(&mut layout, [Op::CloseWindow(2)]);
    assert_eq!(shape(&layout), "[1 3]");
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 625.);
    assert_eq!(r[&3].size.w, 625.);

    more(&mut layout, [Op::CloseWindow(1)]);
    assert_eq!(shape(&layout), "{3}");
    assert_eq!(rects(&layout)[&3].size.w, 1260.);
}

// -- Keyboard moves ---------------------------------------------------------------------------

#[test]
fn moving_swaps_slots_then_leaves_at_the_edge() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let before = rects(&layout);
    assert_eq!(focused(&layout), 1);

    more(&mut layout, [Op::ConsumeOrExpelWindowRight { id: None }]);
    assert_eq!(shape(&layout), "[2 1]");
    assert_eq!(focused(&layout), 1);
    // The slots stay, the windows swap.
    let r = rects(&layout);
    assert_eq!(r[&1], before[&2]);
    assert_eq!(r[&2], before[&1]);

    // At the group's right edge, it leaves into its own column.
    more(&mut layout, [Op::ConsumeOrExpelWindowRight { id: None }]);
    assert_eq!(shape(&layout), "{2} {1}");
    assert_eq!(focused(&layout), 1);

    // Between two plain columns it's niri's stacking again.
    more(&mut layout, [Op::ConsumeOrExpelWindowLeft { id: None }]);
    assert_eq!(shape(&layout), "{2 1}");
}

#[test]
fn moving_from_a_lone_column_into_a_group_merges() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusWindow(3),
        ],
    );
    assert_eq!(shape(&layout), "[1 2] {3}");
    more(&mut layout, [Op::ConsumeOrExpelWindowLeft { id: None }]);
    assert_eq!(shape(&layout), "[1 2 3]");
    assert_eq!(focused(&layout), 3);
    more(&mut layout, [Op::ConsumeOrExpelWindowRight { id: None }]);
    assert_eq!(shape(&layout), "[1 2] {3}");
}

#[test]
fn moving_up_and_down_swaps_stacked_slots() {
    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "(1 2)");
    more(&mut layout, [Op::MoveWindowDown]);
    assert_eq!(shape(&layout), "(2 1)");
    assert_eq!(focused(&layout), 1);
    more(&mut layout, [Op::MoveWindowDown]);
    assert_eq!(shape(&layout), "(2 1)");
    more(&mut layout, [Op::MoveWindowUp]);
    assert_eq!(shape(&layout), "(1 2)");
}

#[test]
fn swap_window_left_right_swaps_slots_and_columns() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2] {3}");
    more(
        &mut layout,
        [Op::SwapWindowInDirection(ScrollDirection::Right)],
    );
    assert_eq!(shape(&layout), "[2 1] {3}");
    // At the group's edge: swap with the next column's window.
    more(
        &mut layout,
        [Op::SwapWindowInDirection(ScrollDirection::Right)],
    );
    assert_eq!(shape(&layout), "[2 3] {1}");
    assert_eq!(focused(&layout), 1);
}

// -- Focus ------------------------------------------------------------------------------------

#[test]
fn focus_goes_through_slots_first() {
    // {0} [1 (2 3)] {4}
    let mut layout = run(
        1.,
        [
            window(0),
            window(1),
            window(2),
            window(3),
            window(4),
            Op::FocusWindow(1),
            Op::SplitWindow { id: None },
            Op::SplitWindowBeside {
                id: Some(3),
                target: 2,
                side: group::Side::Bottom,
            },
        ],
    );
    assert_eq!(shape(&layout), "{0} [1 (2 3)] {4}");

    more(&mut layout, [Op::FocusWindow(1), Op::FocusColumnRight]);
    assert!([2, 3].contains(&focused(&layout)));
    more(&mut layout, [Op::FocusWindow(2), Op::FocusWindowDown]);
    assert_eq!(focused(&layout), 3);
    more(&mut layout, [Op::FocusWindowDown]);
    assert_eq!(focused(&layout), 3);
    more(&mut layout, [Op::FocusColumnRight]);
    assert_eq!(focused(&layout), 4);
    // Coming back enters on the right edge, at the remembered slot.
    more(&mut layout, [Op::FocusColumnLeft]);
    assert_eq!(focused(&layout), 3);
    more(&mut layout, [Op::FocusWindowUp]);
    assert_eq!(focused(&layout), 2);
    more(&mut layout, [Op::FocusWindowUp]);
    assert_eq!(focused(&layout), 2);
    more(&mut layout, [Op::FocusColumnLeft]);
    assert_eq!(focused(&layout), 1);
    more(&mut layout, [Op::FocusColumnLeft]);
    assert_eq!(focused(&layout), 0);
    // Entering from the left: the left edge's slot.
    more(&mut layout, [Op::FocusColumnRight]);
    assert_eq!(focused(&layout), 1);
}

#[test]
fn focus_window_or_workspace_leaves_the_group_at_its_edge() {
    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "(1 2)");
    let ws = layout.active_workspace().unwrap().id();
    more(&mut layout, [Op::FocusWindowOrWorkspaceDown]);
    assert_eq!(focused(&layout), 2);
    assert_eq!(layout.active_workspace().unwrap().id(), ws);
    more(&mut layout, [Op::FocusWindowOrWorkspaceDown]);
    assert_ne!(layout.active_workspace().unwrap().id(), ws);
}

// -- Resizing ---------------------------------------------------------------------------------

#[test]
fn resizing_inside_changes_shares_only() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    more(
        &mut layout,
        [Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(400),
        }],
    );
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 400.);
    assert_eq!(r[&2].size.w, 850.);
    // The group is as wide as before.
    assert_eq!(right(r[&2]) - r[&1].loc.x, 1260.);

    // set-window-height has no stacked split to change here.
    more(
        &mut layout,
        [Op::SetWindowHeight {
            id: Some(1),
            change: SizeChange::SetFixed(300),
        }],
    );
    assert_eq!(rects(&layout)[&1].size.h, 700.);
}

#[test]
fn resizing_outside_scales_the_group() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    more(
        &mut layout,
        [Op::SetColumnWidth(SizeChange::SetProportion(50.))],
    );
    let r = rects(&layout);
    // 625 wide in total, still half and half.
    assert_eq!(right(r[&2]) - r[&1].loc.x, 625.);
    assert_eq!(r[&1].size.w, 308.);
    assert_eq!(r[&2].size.w, 307.);

    // A fixed column width is the whole group's.
    more(
        &mut layout,
        [Op::SetColumnWidth(SizeChange::SetFixed(1000))],
    );
    let r = rects(&layout);
    assert_eq!(right(r[&2]) - r[&1].loc.x, 1000.);

    // Presets too.
    more(&mut layout, [Op::SwitchPresetColumnWidth]);
    let r = rects(&layout);
    assert!(right(r[&2]) - r[&1].loc.x < 1000.);
}

#[test]
fn interactive_resize_inner_edge_moves_the_crevice() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    more(
        &mut layout,
        [
            Op::InteractiveResizeBegin {
                window: 1,
                edges: ResizeEdge::RIGHT,
            },
            Op::InteractiveResizeUpdate {
                window: 1,
                dx: 100.,
                dy: 0.,
            },
            Op::InteractiveResizeEnd { window: 1 },
        ],
    );
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 725.);
    assert_eq!(r[&2].size.w, 525.);

    // The same crevice from the other side.
    more(
        &mut layout,
        [
            Op::InteractiveResizeBegin {
                window: 2,
                edges: ResizeEdge::LEFT,
            },
            Op::InteractiveResizeUpdate {
                window: 2,
                dx: -25.,
                dy: 0.,
            },
            Op::InteractiveResizeEnd { window: 2 },
        ],
    );
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 700.);
    assert_eq!(r[&2].size.w, 550.);
}

#[test]
fn interactive_resize_outer_edge_resizes_the_group() {
    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
    more(
        &mut layout,
        [
            Op::InteractiveResizeBegin {
                window: 2,
                edges: ResizeEdge::RIGHT,
            },
            Op::InteractiveResizeUpdate {
                window: 2,
                dx: 85.,
                dy: 0.,
            },
            Op::InteractiveResizeEnd { window: 2 },
        ],
    );
    let r = rects(&layout);
    assert_eq!(right(r[&2]) - r[&1].loc.x, 710.);
    assert_eq!(r[&1].size.w, 350.);
    assert_eq!(r[&2].size.w, 350.);
}

#[test]
fn edge_resize_zones_offer_inner_edges_only_where_there_is_a_crevice() {
    let layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "(1 2)");
    let r = rects(&layout);
    let output = layout.outputs().next().unwrap().clone();
    let under = |x: f64, y: f64| {
        layout
            .edge_resize_under(&output, Point::from((x, y)))
            .map(|(w, e)| (w.0.id, e))
    };
    // The crevice between the stacked slots.
    let (_, cy) = center(r[&1]);
    let gap_y = bottom(r[&1]) + 3.;
    let hit = under(center(r[&1]).0, gap_y).unwrap();
    assert!(hit.1.intersects(ResizeEdge::TOP_BOTTOM), "{hit:?}");
    // The group's outer top gutter has none.
    assert_eq!(under(center(r[&1]).0, r[&1].loc.y - 4.), None);
    // The outer left/right gaps resize the group.
    let hit = under(r[&1].loc.x - 3., cy).unwrap();
    assert_eq!(hit.1, ResizeEdge::LEFT);
}

// -- Dragging ---------------------------------------------------------------------------------

#[test]
fn drag_into_crevice_inserts_with_equal_shares() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            window(3),
            Op::FocusWindow(1),
        ],
    );
    assert_eq!(shape(&layout), "[1 2] {3}");
    let r = rects(&layout);
    let x = right(r[&1]) + GAPS / 2.;
    let ops = drag(&layout, 3, x, center(r[&1]).1);
    more(&mut layout, ops);
    assert_eq!(shape(&layout), "[1 3 2]");
    let r = rects(&layout);
    let widths: Vec<f64> = [1, 3, 2].iter().map(|id| r[id].size.w).collect();
    assert_eq!(widths, vec![414., 413., 413.]);
    assert_eq!(focused(&layout), 3);
}

#[test]
fn drag_onto_window_swaps() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            window(3),
            Op::FocusWindow(1),
        ],
    );
    let before = rects(&layout);
    let (x, y) = center(before[&2]);
    let ops = drag(&layout, 3, x, y);
    more(&mut layout, ops);
    // 3 takes 2's slot, 2 goes where 3 was: its own column after the group.
    assert_eq!(shape(&layout), "[1 3] {2}");
    assert_eq!(focused(&layout), 3);
}

#[test]
fn drag_within_group_onto_other_window_swaps() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let r = rects(&layout);
    let (x, y) = center(r[&2]);
    let ops = drag(&layout, 1, x, y);
    more(&mut layout, ops);
    assert_eq!(shape(&layout), "[2 1]");
}

#[test]
fn drag_to_column_gap_takes_it_out() {
    let mut layout = run(
        0.25,
        [
            window(0),
            window(1),
            window(2),
            Op::FocusWindow(1),
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "{0} (1 2)");
    let r = rects(&layout);
    // The gap between column 0 and the group.
    let x = r[&1].loc.x - GAPS / 2.;
    let ops = drag(&layout, 2, x, center(r[&1]).1);
    more(&mut layout, ops);
    assert_eq!(shape(&layout), "{0} {2} {1}");
}

#[test]
fn drag_back_onto_own_slot_restores() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
        ],
    );
    let before = rects(&layout);
    let (x, y) = center(before[&2]);

    // While dragging, 2's slot stays vacant.
    let (cx, cy) = center(before[&2]);
    check_ops_on_layout(
        &mut layout,
        [
            Op::InteractiveMoveBegin {
                window: 2,
                output_idx: 1,
                px: cx,
                py: cy,
            },
            Op::InteractiveMoveUpdate {
                window: 2,
                dx: 300.,
                dy: 300.,
                output_idx: 1,
                px: x,
                py: y,
            },
        ],
    );
    assert_eq!(shape(&layout), "[1 _ 3]");
    more(&mut layout, [Op::InteractiveMoveEnd { window: 2 }]);
    assert_eq!(shape(&layout), "[1 2 3]");
    assert_eq!(rects(&layout), before);
}

#[test]
fn closing_a_dragged_window_clears_its_slot() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let (cx, cy) = center(rects(&layout)[&1]);
    more(
        &mut layout,
        [
            Op::InteractiveMoveBegin {
                window: 1,
                output_idx: 1,
                px: cx,
                py: cy,
            },
            Op::InteractiveMoveUpdate {
                window: 1,
                dx: 300.,
                dy: 300.,
                output_idx: 1,
                px: 600.,
                py: 300.,
            },
        ],
    );
    assert_eq!(shape(&layout), "[_ 2]");
    more(&mut layout, [Op::CloseWindow(1)]);
    assert_eq!(shape(&layout), "{2}");
}

// -- Fullscreen, workspaces, IPC --------------------------------------------------------------

#[test]
fn fullscreen_goes_alone_and_comes_back() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
            Op::SetWindowWidth {
                id: Some(2),
                change: SizeChange::SetFixed(500),
            },
        ],
    );
    let before = rects(&layout);
    let id = group_id(&layout, 1);

    more(&mut layout, [Op::FullscreenWindow(2)]);
    assert_eq!(shape(&layout), "[1 3] {2}");
    assert_eq!(rects(&layout)[&2].size, Size::from((1280., 720.)));

    more(&mut layout, [Op::FullscreenWindow(2)]);
    assert_eq!(shape(&layout), "[1 2 3]");
    assert_eq!(group_id(&layout, 1), id);
    // Same slots, same shares.
    let r = rects(&layout);
    for w in [1, 2, 3] {
        assert_eq!(r[&w].size, before[&w].size, "window {w}");
    }
}

#[test]
fn fullscreen_in_group_of_two_comes_back() {
    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    more(&mut layout, [Op::FullscreenWindow(1)]);
    assert_eq!(shape(&layout), "{2} {1}");
    more(&mut layout, [Op::FullscreenWindow(1)]);
    assert_eq!(shape(&layout), "(1 2)");

    // Maximize to edges too.
    more(&mut layout, [Op::MaximizeWindowToEdges { id: Some(2) }]);
    assert_eq!(shape(&layout), "{1} {2}");
    more(&mut layout, [Op::MaximizeWindowToEdges { id: Some(2) }]);
    assert_eq!(shape(&layout), "(1 2)");
}

#[test]
fn moving_a_group_to_another_workspace_keeps_it() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let id = group_id(&layout, 1);
    more(&mut layout, [Op::MoveColumnToWorkspaceDown(true)]);
    assert_eq!(shape(&layout), "[1 2]");
    assert_eq!(group_id(&layout, 2), id);
    assert_eq!(layout.active_workspace().unwrap().id(), {
        let (_, _, ws) = layout
            .workspaces()
            .find(|(_, _, ws)| ws.has_window(&1))
            .unwrap();
        ws.id()
    });

    // A single window moving to another workspace leaves its group.
    more(
        &mut layout,
        [Op::FocusWindow(1), Op::MoveWindowToWorkspaceDown(true)],
    );
    assert_eq!(shape(&layout), "{1}");
    assert_eq!(group_id(&layout, 2), None);
    assert_eq!(group_id(&layout, 1), None);
}

#[test]
fn ipc_reports_groups() {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let ws = layout.active_workspace().unwrap();
    let layouts: HashMap<usize, WindowLayout> = ws
        .tiles_with_ipc_layouts()
        .map(|(tile, layout)| (tile.window().0.id, layout))
        .collect();
    let g1 = layouts[&1].group.clone().unwrap();
    let g2 = layouts[&2].group.clone().unwrap();
    assert_eq!(g1.group_id, g2.group_id);
    assert_eq!((g1.slot, g2.slot, g1.slot_count), (1, 2, 2));
    assert_eq!(layouts[&2].pos_in_scrolling_layout, Some((1, 2)));
    assert_eq!(g2.pos_in_group, (635., 0.));
    assert_eq!(g2.size_in_group, (625., 700.));
    assert_eq!(g1.group_size, (1260., 700.));
    assert_eq!(g1.path.len(), 1);
    assert_eq!(g1.path[0].split, niri_ipc::GroupSplit::SideBySide);
    assert_eq!((g1.path[0].index, g1.path[0].count), (0, 2));
    assert!((g1.path[0].share - 0.5).abs() < 1e-9);

    let groups: Vec<_> = ws.groups_with_render_rects().collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1.size, Size::from((1260., 700.)));
}

// -- Random operations with lots of groups ----------------------------------------------------

/// Ops weighted towards groups: merges, rotations, pop-outs, inserts, drags in the output.
fn group_ops() -> impl Strategy<Value = Vec<Op>> {
    let one = |op: Op| vec![op];
    let drag = (
        1..=5usize,
        0f64..1280.,
        0f64..720.,
        0f64..1280.,
        0f64..720.,
        any::<bool>(),
    )
        .prop_map(|(window, px, py, x, y, end)| {
            let mut ops = vec![
                Op::InteractiveMoveBegin {
                    window,
                    output_idx: 1,
                    px,
                    py,
                },
                Op::InteractiveMoveUpdate {
                    window,
                    dx: 300.,
                    dy: 300.,
                    output_idx: 1,
                    px: x,
                    py: y,
                },
            ];
            if end {
                ops.push(Op::InteractiveMoveEnd { window });
            }
            ops
        });
    let resize = (
        1..=5usize,
        arbitrary_resize_edge(),
        -300f64..300.,
        -300f64..300.,
    )
        .prop_map(|(window, edges, dx, dy)| {
            vec![
                Op::InteractiveResizeBegin { window, edges },
                Op::InteractiveResizeUpdate { window, dx, dy },
                Op::InteractiveResizeEnd { window },
            ]
        });
    prop_oneof![
        6 => any::<Op>().prop_map(one),
        3 => (1..=5usize).prop_map(move |id| one(Op::AddWindow {
            params: TestWindowParams::new(id)
        })),
        4 => proptest::option::of(1..=5usize).prop_map(move |id| one(Op::SplitWindow { id })),
        1 => proptest::option::of(1..=5usize).prop_map(move |id| one(Op::UnsplitWindow { id })),
        2 => (proptest::option::of(1..=5usize), 1..=5usize, arbitrary_split_side())
            .prop_map(move |(id, target, side)| one(Op::SplitWindowBeside { id, target, side })),
        2 => (1..=5usize).prop_map(move |id| one(Op::Communicate(id))),
        // Time passing, past the transaction deadline of held slots too.
        2 => (0..400i32).prop_map(move |msec_delta| one(Op::AdvanceAnimations { msec_delta })),
        2 => drag,
        1 => resize,
        1 => (1..=5usize).prop_map(move |id| one(Op::FullscreenWindow(id))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if std::env::var_os("RUN_SLOW_TESTS").is_none() {
            64
        } else {
            ProptestConfig::default().cases
        },
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_group_operations_keep_invariants(
        ops in prop::collection::vec(group_ops(), 0..50),
        wide in any::<bool>(),
        animated in any::<bool>(),
    ) {
        let mut all = vec![Op::AddOutput(1)];
        all.extend(ops.into_iter().flatten());
        all.push(Op::CompleteAnimations);
        let width = if wide { 1. } else { 0.5 };
        let options = if animated { anim_options(width) } else { options(width) };
        check_ops_with_options(options, all);
    }
}

/// Ops weighted towards the ways a window leaves a group (closing, unmapping, going to another
/// workspace or output, floating, fullscreen, maximize, consume/expel, moves, tabbed, drags that
/// end elsewhere or never), interleaved with merges, so a group left with one window shows up.
fn group_exit_ops() -> impl Strategy<Value = Vec<Op>> {
    let one = |op: Op| vec![op];
    let id = || 1..=5usize;
    let drag = (
        id(),
        (0f64..1280., 0f64..720.),
        (0f64..1280., 0f64..720.),
        1..=2usize,
        0..3u8,
    )
        .prop_map(|(window, (px, py), (x, y), output_idx, end)| {
            let mut ops = vec![
                Op::InteractiveMoveBegin {
                    window,
                    output_idx: 1,
                    px,
                    py,
                },
                Op::InteractiveMoveUpdate {
                    window,
                    dx: 300.,
                    dy: 300.,
                    output_idx,
                    px: x,
                    py: y,
                },
            ];
            match end {
                0 => ops.push(Op::InteractiveMoveEnd { window }),
                1 => ops.push(Op::CloseWindow(window)),
                _ => (),
            }
            ops
        });
    prop_oneof![
        4 => any::<Op>().prop_map(one),
        4 => id().prop_map(move |id| one(Op::AddWindow {
            params: TestWindowParams::new(id)
        })),
        6 => proptest::option::of(id()).prop_map(move |id| one(Op::SplitWindow { id })),
        1 => proptest::option::of(id()).prop_map(move |id| one(Op::UnsplitWindow { id })),
        2 => (proptest::option::of(id()), id(), arbitrary_split_side())
            .prop_map(move |(id, target, side)| one(Op::SplitWindowBeside { id, target, side })),
        3 => id().prop_map(move |id| one(Op::CloseWindow(id))),
        2 => (proptest::option::of(id()), 0..=2usize).prop_map(move |(window_id, workspace_idx)| {
            one(Op::MoveWindowToWorkspace {
                window_id,
                workspace_idx,
            })
        }),
        1 => (proptest::option::of(id()), 1..=2usize).prop_map(move |(window_id, output_id)| {
            one(Op::MoveWindowToOutput {
                window_id,
                output_id,
                target_ws_idx: None,
            })
        }),
        2 => proptest::option::of(id()).prop_map(move |id| one(Op::ToggleWindowFloating { id })),
        1 => id().prop_map(move |id| one(Op::FullscreenWindow(id))),
        1 => proptest::option::of(id()).prop_map(move |id| one(Op::MaximizeWindowToEdges { id })),
        1 => Just(one(Op::MaximizeColumn)),
        2 => proptest::option::of(id())
            .prop_map(move |id| one(Op::ConsumeOrExpelWindowLeft { id })),
        2 => proptest::option::of(id())
            .prop_map(move |id| one(Op::ConsumeOrExpelWindowRight { id })),
        1 => Just(one(Op::ConsumeWindowIntoColumn)),
        1 => Just(one(Op::ExpelWindowFromColumn)),
        1 => Just(one(Op::MoveColumnLeft)),
        1 => Just(one(Op::MoveColumnRight)),
        1 => Just(one(Op::MoveWindowUp)),
        1 => Just(one(Op::MoveWindowDown)),
        1 => Just(one(Op::ToggleColumnTabbedDisplay)),
        1 => Just(one(Op::FocusColumnLeft)),
        1 => Just(one(Op::FocusColumnRight)),
        1 => Just(one(Op::FocusWorkspaceDown)),
        1 => Just(one(Op::FocusWorkspaceUp)),
        1 => Just(one(Op::AddOutput(2))),
        1 => Just(one(Op::RemoveOutput(2))),
        2 => id().prop_map(move |id| one(Op::Communicate(id))),
        2 => (0..400i32).prop_map(move |msec_delta| one(Op::AdvanceAnimations { msec_delta })),
        2 => drag,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if std::env::var_os("RUN_SLOW_TESTS").is_none() {
            64
        } else {
            ProptestConfig::default().cases
        },
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_group_exits_keep_invariants(
        ops in prop::collection::vec(group_exit_ops(), 0..60),
        wide in any::<bool>(),
        animated in any::<bool>(),
    ) {
        let mut all = vec![Op::AddOutput(1)];
        all.extend(ops.into_iter().flatten());
        all.push(Op::CompleteAnimations);
        let width = if wide { 1. } else { 0.5 };
        let options = if animated { anim_options(width) } else { options(width) };
        check_ops_with_options(options, all);
    }
}

#[test]
fn zero_width_column_merges_without_panicking() {
    let ops = [
        Op::AddOutput(1),
        window(1),
        Op::SetColumnWidth(SizeChange::SetProportion(0.)),
        window(3),
        Op::SplitWindow { id: None },
    ];
    check_ops_with_options(options(0.5), ops);
}

#[test]
fn swapping_slots_with_min_widths_keeps_column_data() {
    let mut p5 = TestWindowParams::new(5);
    p5.bbox = Rectangle::from_size(Size::from((1, 1)));
    p5.min_max_size = (Size::from((82, 0)), Size::from((82, 0)));
    let mut rules = ResolvedWindowRules::default();
    rules.border.on = true;
    p5.rules = Some(rules);
    // Found by random_group_operations_keep_invariants: the swap puts the min-width window
    // side by side, which widens the group.
    let ops = vec![
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow { params: p5 },
        Op::AddWindow {
            params: TestWindowParams::new(4),
        },
        Op::MoveColumnLeftOrToMonitorLeft(1),
        Op::SplitWindow { id: None },
        Op::FocusColumnFirst,
        Op::MoveWorkspaceDown,
        Op::SetColumnWidth(SizeChange::SetFixed(0)),
        Op::SplitWindowBeside {
            id: None,
            target: 4,
            side: group::Side::Left,
        },
        Op::MoveWindowDownOrToWorkspaceDown,
    ];
    let mut layout = Layout::with_options(Clock::with_time(Duration::ZERO), options(0.5));
    for op in ops {
        op.apply(&mut layout);
        layout.verify_invariants();
    }
}

#[test]
fn floating_and_tabbed_leave_groups_alone() {
    let mut layout = run(
        1.,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::FocusColumnLast,
            Op::SplitWindow { id: None },
        ],
    );
    // A group can't be tabbed.
    more(&mut layout, [Op::ToggleColumnTabbedDisplay]);
    assert_eq!(shape(&layout), "[1 2 3]");

    // Floating takes the window out of its group; back to tiling it's a column of its own.
    more(&mut layout, [Op::ToggleWindowFloating { id: Some(2) }]);
    assert_eq!(shape(&layout), "[1 3]");
    more(&mut layout, [Op::ToggleWindowFloating { id: Some(2) }]);
    assert!(shape(&layout).contains("{2}"), "{}", shape(&layout));
    assert!(shape(&layout).contains("[1 3]"), "{}", shape(&layout));
}

// -- Animations -------------------------------------------------------------------------------

/// `options()` with animations on: window movement, window resize and the view all go linearly
/// over 1 s, so the state mid-way is easy to tell.
fn anim_options(default_width: f64) -> Options {
    const LINEAR: Kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });
    let mut options = options(default_width);
    options.animations.off = false;
    options.animations.window_resize.anim.kind = LINEAR;
    options.animations.window_movement.0.kind = LINEAR;
    options.animations.horizontal_view_movement.0.kind = LINEAR;
    options
}

fn run_animated(
    options: Options,
    width: f64,
    ops: impl IntoIterator<Item = Op>,
) -> Layout<TestWindow> {
    WIDTH.with(|w| w.set(width));
    let mut all: Vec<Op> = vec![Op::AddOutput(1)];
    all.extend(with_widths(ops));
    all.extend(settle());
    check_ops_with_options(options, all)
}

/// Applies `ops` without letting the windows commit or the time pass.
fn apply(layout: &mut Layout<TestWindow>, ops: impl IntoIterator<Item = Op>) {
    check_ops_on_layout(layout, ops);
}

fn advance(layout: &mut Layout<TestWindow>, msec: i32) {
    check_ops_on_layout(layout, [Op::AdvanceAnimations { msec_delta: msec }]);
}

fn commit(layout: &mut Layout<TestWindow>, ids: impl IntoIterator<Item = usize>) {
    check_ops_on_layout(layout, ids.into_iter().map(Op::Communicate));
}

/// Tile rectangles as drawn now (animated position and size) by window id.
fn drawn(layout: &Layout<TestWindow>) -> HashMap<usize, Rectangle<f64, Logical>> {
    layout
        .active_workspace()
        .unwrap()
        .tiles_with_render_positions()
        .filter(|(_, _, visible)| *visible)
        .map(|(tile, pos, _)| {
            (
                tile.window().0.id,
                Rectangle::new(pos, tile.animated_tile_size()),
            )
        })
        .collect()
}

fn bounds(rects: impl IntoIterator<Item = Rectangle<f64, Logical>>) -> Rectangle<f64, Logical> {
    rects.into_iter().reduce(|a, b| a.merge(b)).unwrap()
}

fn assert_inside(r: Rectangle<f64, Logical>, area: Rectangle<f64, Logical>, what: &str) {
    // Positions and sizes are rounded to physical pixels each.
    let eps = 1.;
    assert!(
        r.loc.x >= area.loc.x - eps
            && r.loc.y >= area.loc.y - eps
            && right(r) <= right(area) + eps
            && bottom(r) <= bottom(area) + eps,
        "{what}: {r:?} sticks out of the group {area:?}"
    );
}

/// The rotate in the stacked group of windows 1 and 2 (1 focused), from a settled state.
fn stacked_pair_animated() -> Layout<TestWindow> {
    let layout = run_animated(
        anim_options(0.25),
        0.25,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "(1 2)");
    layout
}

#[test]
fn merge_moves_the_view_at_once() {
    // A lone column is centred (as in Mike's config). After window 1 merges into window 2's
    // column, the group is the only column, so the view must head for the centre right away,
    // not once the windows commit (meanwhile the merged window moved without the view and its
    // neighbour went past its place and back).
    let mut options = anim_options(0.25);
    options.layout.always_center_single_column = true;
    let mut layout = run_animated(options, 0.25, [window(1), window(2), Op::FocusColumnFirst]);

    apply(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "(1 2)");
    let view = layout.active_workspace().unwrap().scrolling().view_offset();
    assert!(view.is_animation_ongoing(), "the view doesn't move");
    let target = view.target();

    // The commits don't change where the view goes.
    more(&mut layout, []);
    let view = layout.active_workspace().unwrap().scrolling().view_offset();
    assert_eq!(view.target(), target);
    // Centred.
    let r = rects(&layout);
    let group = bounds([r[&1], r[&2]]);
    // (Rounded to a physical pixel.)
    assert!((group.loc.x - (1280. - group.size.w) / 2.).abs() <= 0.5);
}

#[test]
fn merge_glides_from_where_both_windows_were() {
    let mut layout = run_animated(
        anim_options(0.25),
        0.25,
        [
            window(1),
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            window(2),
            Op::SetColumnWidth(SizeChange::SetFixed(200)),
            window(3),
            Op::SetColumnWidth(SizeChange::SetFixed(300)),
            Op::FocusColumnFirst,
        ],
    );
    let before = drawn(&layout);

    apply(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "(1 2) {3}");
    // Nothing jumps: both windows start where they were, and the column after the group, whose
    // place doesn't change, stays put all along (the source column's removal and the group's
    // growth over its space cancel out).
    assert_eq!(drawn(&layout), before);
    commit(&mut layout, [1, 2]);
    for _ in 0..10 {
        advance(&mut layout, 100);
        assert_eq!(drawn(&layout)[&3], before[&3]);
    }
    more(&mut layout, []);
    let r = rects(&layout);
    assert_eq!(r[&3], before[&3]);
    assert_eq!(r[&1].loc.x, before[&1].loc.x);
    assert_eq!(right(r[&2]), right(before[&2]));
}

#[test]
fn rotate_holds_resizing_slots_until_their_commit() {
    let mut layout = stacked_pair_animated();
    let before = drawn(&layout);
    let group = bounds(before.values().copied());

    apply(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "[1 2]");
    // Until the windows commit their new sizes, nothing moves: window 2 doesn't jump to its new
    // slot at its old size, sticking out of the group.
    advance(&mut layout, 100);
    assert_eq!(drawn(&layout), before);

    // Then each slot moves together with its resize, inside the group all the way.
    commit(&mut layout, [1, 2]);
    assert_eq!(drawn(&layout), before);
    advance(&mut layout, 500);
    let mid = drawn(&layout);
    assert_ne!(
        mid[&2].loc, before[&2].loc,
        "window 2 doesn't move after its commit"
    );
    for _ in 0..6 {
        for (id, r) in drawn(&layout) {
            assert_inside(r, group, &format!("window {id}"));
        }
        advance(&mut layout, 100);
    }
    more(&mut layout, []);
    assert_eq!(drawn(&layout), rects(&layout));
    assert_eq!(bounds(rects(&layout).into_values()), group);
}

#[test]
fn held_slot_moves_anyway_at_the_transaction_deadline() {
    let mut layout = stacked_pair_animated();
    let before = drawn(&layout);

    apply(&mut layout, [Op::SplitWindow { id: None }]);
    // The windows never commit. The layout keeps asking for frames, to notice the deadline.
    assert!(layout.are_animations_ongoing(None));
    advance(&mut layout, 250);
    assert_eq!(drawn(&layout), before);

    // Past the 300 ms of the configure's transaction, window 2 goes to its slot without waiting.
    advance(&mut layout, 100);
    advance(&mut layout, 500);
    let mid = drawn(&layout)[&2];
    assert_ne!(
        mid.loc, before[&2].loc,
        "window 2 still waits for its commit"
    );
    advance(&mut layout, 600);
    let moved = drawn(&layout)[&2];
    // At its old size until it commits.
    assert_eq!(moved.size, before[&2].size);

    // The late commit only resizes it.
    more(&mut layout, []);
    let r = rects(&layout)[&2];
    assert_eq!(r.loc, moved.loc);
    assert_ne!(r.size, before[&2].size);
}

#[test]
fn rotating_back_before_the_commit_acts_at_once() {
    let mut layout = stacked_pair_animated();
    let before = drawn(&layout);

    apply(&mut layout, [Op::SplitWindow { id: None }]);
    advance(&mut layout, 20);
    // A second Mod+S while the first waits for the windows: back to stacked from what is drawn
    // (still the stacked layout), at once, with nothing left to wait for.
    apply(&mut layout, [Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "(1 2)");
    assert_eq!(drawn(&layout), before);
    let ws = layout.active_workspace().unwrap();
    for (tile, _, _) in ws.tiles_with_render_positions() {
        assert_eq!(tile.render_offset(), Point::from((0., 0.)));
    }
    assert!(!layout.are_animations_ongoing(None));

    // The commits of the sizes asked for last change nothing.
    commit(&mut layout, [1, 2]);
    advance(&mut layout, 500);
    assert_eq!(drawn(&layout), before);
}

#[test]
fn swapping_equal_slots_moves_at_once() {
    let mut layout = stacked_pair_animated();
    let before = drawn(&layout);
    assert_eq!(before[&1].size, before[&2].size);

    // The slots keep their sizes, so the windows don't wait for a commit.
    apply(&mut layout, [Op::MoveWindowDown]);
    assert_eq!(shape(&layout), "(2 1)");
    advance(&mut layout, 500);
    let mid = drawn(&layout);
    let half = (before[&1].loc.y + before[&2].loc.y) / 2.;
    assert!((mid[&1].loc.y - half).abs() <= 0.5, "{mid:?}");
    assert!((mid[&2].loc.y - half).abs() <= 0.5, "{mid:?}");
    advance(&mut layout, 500);
    let r = drawn(&layout);
    assert_eq!(r[&1], before[&2]);
    assert_eq!(r[&2], before[&1]);
}

// -- Resizes wait for every window of the group -----------------------------------------------

/// The windows' sizes as last requested, by window id.
fn requested(layout: &Layout<TestWindow>) -> HashMap<usize, Size<i32, Logical>> {
    layout
        .active_workspace()
        .unwrap()
        .tiles_with_render_positions()
        .map(|(tile, _, _)| (tile.window().0.id, tile.window().requested_size().unwrap()))
        .collect()
}

/// Asserts that the drawn slots of a side-by-side pair `a | b` tile the group: `gap` between
/// them, no more and no less (rounded to physical pixels each).
fn assert_neighbours(r: &HashMap<usize, Rectangle<f64, Logical>>, a: usize, b: usize, what: &str) {
    let gap = r[&b].loc.x - right(r[&a]);
    assert!(
        (gap - GAPS).abs() <= 1.,
        "{what}: {gap} px between windows {a} and {b}: {r:?}"
    );
}

/// `[1 2]` side by side at full width (1260 px), settled.
fn side_by_side_pair() -> Layout<TestWindow> {
    let layout = run(
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2]");
    layout
}

fn crevice_drag_begin() -> [Op; 1] {
    [Op::InteractiveResizeBegin {
        window: 1,
        edges: ResizeEdge::RIGHT,
    }]
}

fn crevice_drag_to(dx: f64) -> [Op; 1] {
    [Op::InteractiveResizeUpdate {
        window: 1,
        dx,
        dy: 0.,
    }]
}

#[test]
fn dragging_a_divider_moves_both_slots_on_the_same_commit() {
    let mut layout = side_by_side_pair();
    let before = rects(&layout);

    apply(&mut layout, crevice_drag_begin());
    apply(&mut layout, crevice_drag_to(100.));
    // Both windows are asked for their new sizes, in one transaction.
    let req = requested(&layout);
    assert_eq!(req[&1].w, 725);
    assert_eq!(req[&2].w, 525);
    // Until they commit, the slots stay where the buffers are: window 2 doesn't move to its new
    // place at its old size (on top of the group's edge), leaving a gap after window 1.
    assert_eq!(rects(&layout), before);

    // A fast window that committed waits for its slow neighbour. (In niri its commit is held by
    // the transaction until the neighbour's; the test windows' commits apply at once.)
    commit(&mut layout, [1]);
    let r = rects(&layout);
    assert_eq!(r[&2], before[&2]);
    assert_eq!(r[&1].loc, before[&1].loc);

    // Then both are at their new places, on the same frame.
    commit(&mut layout, [2]);
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 725.);
    assert_eq!(r[&2].size.w, 525.);
    assert_neighbours(&r, 1, 2, "after both commits");
    assert_eq!(right(r[&2]), right(before[&2]));
    apply(&mut layout, [Op::InteractiveResizeEnd { window: 1 }]);
}

#[test]
fn a_drag_sends_new_sizes_only_once_the_group_caught_up() {
    let mut layout = side_by_side_pair();

    apply(&mut layout, crevice_drag_begin());
    apply(&mut layout, crevice_drag_to(100.));
    commit(&mut layout, [1]);
    // The pointer moves on while window 2 still draws its new size: no new sizes go out (they'd
    // queue up behind window 2's, and window 1 would run ahead of it).
    apply(&mut layout, crevice_drag_to(150.));
    apply(&mut layout, crevice_drag_to(200.));
    let req = requested(&layout);
    assert_eq!(req[&1].w, 725);
    assert_eq!(req[&2].w, 525);

    // Once window 2 committed, the newest position goes out, to both.
    commit(&mut layout, [2]);
    let req = requested(&layout);
    assert_eq!(req[&1].w, 825);
    assert_eq!(req[&2].w, 425);
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 725.);
    assert_neighbours(&r, 1, 2, "while the next sizes are on their way");

    commit(&mut layout, [2, 1]);
    apply(&mut layout, [Op::InteractiveResizeEnd { window: 1 }]);
    more(&mut layout, []);
    let r = rects(&layout);
    assert_eq!(r[&1].size.w, 825.);
    assert_eq!(r[&2].size.w, 425.);
    assert_neighbours(&r, 1, 2, "settled");
}

#[test]
fn a_hung_window_holds_its_neighbours_until_the_transaction_deadline() {
    let mut options = options(1.);
    options.animations.off = false;
    let mut layout = run_animated(
        options,
        1.,
        [
            window(1),
            window(2),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
        ],
    );
    let before = rects(&layout);

    apply(&mut layout, crevice_drag_begin());
    apply(&mut layout, crevice_drag_to(100.));
    commit(&mut layout, [1]);
    advance(&mut layout, 250);
    assert_eq!(rects(&layout)[&2], before[&2]);
    apply(&mut layout, crevice_drag_to(200.));
    assert_eq!(requested(&layout)[&1].w, 725);

    // Past the 300 ms of the transaction, the group stops waiting: the slots go to the layout
    // asked for, and the newest drag position goes out.
    advance(&mut layout, 100);
    assert_ne!(rects(&layout)[&2].loc, before[&2].loc);
    assert_eq!(requested(&layout)[&1].w, 825);
    apply(&mut layout, [Op::InteractiveResizeEnd { window: 1 }]);
}

/// `[1 2]` side by side at half width with column 3 after it, settled, animated.
fn side_by_side_pair_and_a_column() -> Layout<TestWindow> {
    let layout = run_animated(
        anim_options(0.25),
        0.25,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::SplitWindow { id: None },
        ],
    );
    assert_eq!(shape(&layout), "[1 2] {3}");
    layout
}

#[test]
fn group_width_change_shows_with_the_commits() {
    let mut layout = side_by_side_pair_and_a_column();
    let before = drawn(&layout);

    // Mod+R: the group goes from 1/2 to 2/3, both slots get wider.
    apply(&mut layout, [Op::SwitchPresetColumnWidth]);
    let req = requested(&layout);
    assert!(req[&1].w > before[&1].size.w as i32);
    assert!(req[&2].w > before[&2].size.w as i32);
    // Nothing moves before the windows commit: not window 2 (it would leave a gap after window
    // 1 at its old size), not column 3 (it would leave one after the group).
    advance(&mut layout, 100);
    assert_eq!(drawn(&layout), before);
    // One window committed: the group waits for the other. (In niri the transaction holds the
    // first commit until the second; the test windows' commits apply at once, so no time passes
    // in between here.)
    commit(&mut layout, [2]);
    let r = drawn(&layout);
    assert_eq!(r[&2].loc, before[&2].loc);
    assert_eq!(r[&3], before[&3]);

    // Then the slots and column 3 move with the resizes, the gaps holding on every frame.
    commit(&mut layout, [1]);
    let mut moved = false;
    for _ in 0..11 {
        let r = drawn(&layout);
        assert_neighbours(&r, 1, 2, "slots");
        assert_neighbours(&r, 2, 3, "group and column 3");
        moved |= r[&3].loc.x != before[&3].loc.x;
        advance(&mut layout, 100);
    }
    assert!(moved, "column 3 never moved");
    more(&mut layout, []);
    let r = rects(&layout);
    assert_eq!(drawn(&layout), r);
    assert_neighbours(&r, 1, 2, "settled slots");
    assert_neighbours(&r, 2, 3, "settled column 3");
}

#[test]
fn group_resizes_ask_every_window_in_one_transaction() {
    fn assert_one_transaction(layout: &Layout<TestWindow>, what: &str) {
        let ws = layout.active_workspace().unwrap();
        let txns: Vec<_> = ws
            .tiles_with_render_positions()
            .map(|(tile, _, _)| (tile.window().0.id, tile.window().transaction()))
            .collect();
        let (first_id, first) = &txns[0];
        let first = first
            .as_ref()
            .unwrap_or_else(|| panic!("{what}: window {first_id} has no transaction"));
        for (id, txn) in &txns[1..] {
            let txn = txn
                .as_ref()
                .unwrap_or_else(|| panic!("{what}: window {id} has no transaction"));
            assert!(
                first.is_same(txn),
                "{what}: windows {first_id} and {id} are in different transactions"
            );
        }
    }

    let mut layout = run(
        0.25,
        [
            window(1),
            window(2),
            window(3),
            Op::FocusColumnFirst,
            Op::SplitWindow { id: None },
            Op::SplitWindow { id: None },
            // [1 2] with 3 beside 2 below it: [1 (2 3)].
            Op::SplitWindowBeside {
                id: Some(3),
                target: 2,
                side: group::Side::Bottom,
            },
        ],
    );
    assert_eq!(shape(&layout), "[1 (2 3)]");
    commit(&mut layout, [1, 2, 3]);

    apply(&mut layout, [Op::SwitchPresetColumnWidth]);
    assert_one_transaction(&layout, "Mod+R");
    more(&mut layout, []);

    apply(
        &mut layout,
        [
            Op::InteractiveResizeBegin {
                window: 1,
                edges: ResizeEdge::RIGHT,
            },
            Op::InteractiveResizeUpdate {
                window: 1,
                dx: 50.,
                dy: 0.,
            },
        ],
    );
    assert_one_transaction(&layout, "divider drag");
    apply(&mut layout, [Op::InteractiveResizeEnd { window: 1 }]);
    more(&mut layout, []);

    apply(
        &mut layout,
        [Op::FocusWindow(1), Op::SplitWindow { id: None }],
    );
    assert_one_transaction(&layout, "rotate");
    more(&mut layout, []);

    apply(&mut layout, [Op::CloseWindow(2)]);
    more(&mut layout, []);
}
