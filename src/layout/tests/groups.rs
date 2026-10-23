//! Split groups (docs/wiki/Development:-Split-Groups.md).
//!
//! The test output is 1280 x 720 with 10 px gaps: a half-width column is 625 x 700 (tall, so
//! merges stack), a full-width one 1260 x 700 (wide, so merges go side by side).

use std::collections::HashMap;

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
        0.5,
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
    // The group keeps the target column's width; the window takes the top half.
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

#[test]
fn split_alone_does_nothing() {
    let layout = run(1., [window(1), Op::SplitWindow { id: None }]);
    assert_eq!(shape(&layout), "{1}");
}

#[test]
fn split_again_rotates_and_back() {
    let mut layout = run(
        0.5,
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
        0.5,
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
        0.5,
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
        0.5,
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
        0.5,
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
        0.5,
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
        0.5,
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
    ) {
        let mut all = vec![Op::AddOutput(1)];
        all.extend(ops.into_iter().flatten());
        all.push(Op::CompleteAnimations);
        check_ops_with_options(options(if wide { 1. } else { 0.5 }), all);
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
