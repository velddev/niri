//! Resize zones in the gaps next to tiles (Layout::edge_resize_under()).

use std::collections::HashMap;

use super::*;
use crate::layout::workspace::edge_zone_hit;

const GAPS: f64 = 10.;

fn options() -> Options {
    Options {
        layout: niri_config::Layout {
            gaps: GAPS,
            ..Default::default()
        },
        animations: niri_config::Animations {
            off: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Runs `ops`, then completes all animations so render positions are final.
fn layout_with(ops: impl IntoIterator<Item = Op>) -> Layout<TestWindow> {
    let ops = ops.into_iter().chain([Op::CompleteAnimations]);
    check_ops_with_options(options(), ops)
}

fn window(id: usize) -> Op {
    Op::AddWindow {
        params: TestWindowParams::new(id),
    }
}

/// Tile rectangles of the active workspace by window id (output at 0,0, so output coordinates).
fn rects(layout: &Layout<TestWindow>) -> HashMap<usize, Rectangle<f64, Logical>> {
    layout
        .active_workspace()
        .unwrap()
        .tiles_with_render_positions()
        .filter(|(_, _, visible)| *visible)
        .map(|(tile, pos, _)| (tile.window().0.id, Rectangle::new(pos, tile.tile_size())))
        .collect()
}

fn under(layout: &Layout<TestWindow>, x: f64, y: f64) -> Option<(usize, ResizeEdge)> {
    let output = layout.outputs().next().unwrap().clone();
    layout
        .edge_resize_under(&output, Point::from((x, y)))
        .map(|(win, edges)| (win.0.id, edges))
}

fn right(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.x + r.size.w
}

fn bottom(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.y + r.size.h
}

fn mid_y(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.y + r.size.h / 2.
}

fn mid_x(r: Rectangle<f64, Logical>) -> f64 {
    r.loc.x + r.size.w / 2.
}

/// Every point of the 1280x720 test output, every 5 px.
fn any_zone_on_output(layout: &Layout<TestWindow>) -> Option<(f64, f64, usize, ResizeEdge)> {
    (0..=256)
        .flat_map(|x| (0..=144).map(move |y| (x as f64 * 5., y as f64 * 5.)))
        .find_map(|(x, y)| under(layout, x, y).map(|(id, e)| (x, y, id, e)))
}

#[test]
fn zone_geometry() {
    let rect = Rectangle::new(Point::from((100., 100.)), Size::from((200., 100.)));

    assert_eq!(edge_zone_hit(rect, Point::from((150., 150.)), 10.), None);
    assert_eq!(
        edge_zone_hit(rect, Point::from((95., 150.)), 10.),
        Some((ResizeEdge::LEFT, 5.))
    );
    // The right and bottom edges are outside the rectangle, like Rectangle::contains().
    assert_eq!(
        edge_zone_hit(rect, Point::from((300., 150.)), 10.),
        Some((ResizeEdge::RIGHT, 0.))
    );
    assert_eq!(
        edge_zone_hit(rect, Point::from((150., 210.)), 10.),
        Some((ResizeEdge::BOTTOM, 10.))
    );
    assert_eq!(edge_zone_hit(rect, Point::from((150., 211.)), 10.), None);
    assert_eq!(edge_zone_hit(rect, Point::from((89., 150.)), 10.), None);

    let (edges, dist) = edge_zone_hit(rect, Point::from((96., 97.)), 10.).unwrap();
    assert_eq!(edges, ResizeEdge::TOP_LEFT);
    assert_eq!(dist, 5.);
    assert_eq!(edge_zone_hit(rect, Point::from((95., 89.)), 10.), None);
}

#[test]
fn gap_between_columns_picks_nearer_edge() {
    let layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::FocusColumnFirst,
    ]);
    let r = rects(&layout);
    let (r1, r2) = (r[&1], r[&2]);
    assert_eq!(r2.loc.x - right(r1), GAPS);

    let y = mid_y(r1);
    assert_eq!(
        under(&layout, right(r1) + 2., y),
        Some((1, ResizeEdge::RIGHT))
    );
    assert_eq!(
        under(&layout, r2.loc.x - 2., y),
        Some((2, ResizeEdge::LEFT))
    );

    // The screen-edge gutter.
    assert!(r1.loc.x >= GAPS);
    assert_eq!(
        under(&layout, r1.loc.x - 3., y),
        Some((1, ResizeEdge::LEFT))
    );
}

#[test]
fn single_window_column_has_no_vertical_zones() {
    let layout = layout_with([Op::AddOutput(1), window(1), Op::Communicate(1)]);
    let r1 = rects(&layout)[&1];
    let x = mid_x(r1);

    assert_eq!(under(&layout, x, r1.loc.y - 3.), None);
    assert_eq!(under(&layout, x, bottom(r1) + 3.), None);

    // Corners keep only the horizontal part.
    assert_eq!(
        under(&layout, r1.loc.x - 3., r1.loc.y - 3.),
        Some((1, ResizeEdge::LEFT))
    );
    assert_eq!(
        under(&layout, right(r1) + 3., bottom(r1) + 3.),
        Some((1, ResizeEdge::RIGHT))
    );
}

#[test]
fn zone_ends_after_gap_width() {
    // A single column leaves the rest of the output empty to its right.
    let layout = layout_with([Op::AddOutput(1), window(1), Op::Communicate(1)]);
    let r1 = rects(&layout)[&1];
    let y = mid_y(r1);

    assert_eq!(
        under(&layout, right(r1) + GAPS - 1., y),
        Some((1, ResizeEdge::RIGHT))
    );
    assert_eq!(under(&layout, right(r1) + GAPS + 1., y), None);
}

#[test]
fn stacked_windows_share_horizontal_gap() {
    let layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::Communicate(1),
        Op::Communicate(2),
    ]);
    let r = rects(&layout);
    let (mut t, mut l) = (1, 2);
    if r[&t].loc.y > r[&l].loc.y {
        (t, l) = (l, t);
    }
    let (top, low) = (r[&t], r[&l]);
    assert_eq!(low.loc.y - bottom(top), GAPS);
    let x = mid_x(top);

    assert_eq!(
        under(&layout, x, bottom(top) + 2.),
        Some((t, ResizeEdge::BOTTOM))
    );
    assert_eq!(
        under(&layout, x, low.loc.y - 2.),
        Some((l, ResizeEdge::TOP))
    );
    assert_eq!(
        under(&layout, x, bottom(low) + 3.),
        Some((l, ResizeEdge::BOTTOM))
    );

    // interactive_resize_update() ignores the topmost window's top edge.
    assert_eq!(under(&layout, x, top.loc.y - 3.), None);
    assert_eq!(
        under(&layout, top.loc.x - 3., top.loc.y - 3.),
        Some((t, ResizeEdge::LEFT))
    );

    assert_eq!(
        under(&layout, right(low) + 3., bottom(low) + 3.),
        Some((l, ResizeEdge::BOTTOM_RIGHT))
    );
}

#[test]
fn tabbed_column_has_no_vertical_zones() {
    let layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetColumnDisplay(ColumnDisplay::Tabbed),
        Op::Communicate(1),
        Op::Communicate(2),
    ]);
    let r = rects(&layout);
    assert_eq!(r.len(), 1);
    let (&id, &rect) = r.iter().next().unwrap();

    assert_eq!(under(&layout, mid_x(rect), bottom(rect) + 3.), None);
    assert_eq!(
        under(&layout, right(rect) + 3., mid_y(rect)),
        Some((id, ResizeEdge::RIGHT))
    );
}

#[test]
fn fullscreen_has_no_zones() {
    let layout = layout_with([
        Op::AddOutput(1),
        window(1),
        Op::FullscreenWindow(1),
        Op::Communicate(1),
    ]);
    assert_eq!(any_zone_on_output(&layout), None);

    // A normal window next to a fullscreen one keeps its zones; the fullscreen one has none.
    let layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::FullscreenWindow(1),
        Op::Communicate(1),
        Op::Communicate(2),
    ]);
    let zones: Vec<_> = (0..=256)
        .flat_map(|x| (0..=144).map(move |y| (x as f64 * 5., y as f64 * 5.)))
        .filter_map(|(x, y)| under(&layout, x, y))
        .collect();
    assert!(zones.iter().all(|(id, _)| *id == 2), "{zones:?}");
}

#[test]
fn overview_has_no_zones() {
    let mut layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::Communicate(1),
        Op::Communicate(2),
    ]);
    assert!(any_zone_on_output(&layout).is_some());

    layout.toggle_overview();
    assert!(layout.is_overview_open());
    assert_eq!(any_zone_on_output(&layout), None);
}

#[test]
fn fixed_width_window_has_no_horizontal_zones() {
    let mut params = TestWindowParams::new(1);
    params.min_max_size = (Size::from((300, 0)), Size::from((300, 0)));
    let layout = layout_with([
        Op::AddOutput(1),
        Op::AddWindow { params },
        window(2),
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::Communicate(1),
        Op::Communicate(2),
    ]);
    let r = rects(&layout);
    let (fixed, other) = (r[&1], r[&2]);

    assert_eq!(under(&layout, right(fixed) + 3., mid_y(fixed)), None);
    assert_eq!(
        under(&layout, mid_x(fixed), bottom(fixed) + 2.),
        Some((1, ResizeEdge::BOTTOM))
    );
    assert_eq!(
        under(&layout, right(other) + 3., mid_y(other)),
        Some((2, ResizeEdge::RIGHT))
    );
}

#[test]
fn floating_window_has_all_edges() {
    let mut params = TestWindowParams::new(1);
    params.is_floating = true;
    let layout = layout_with([
        Op::AddOutput(1),
        Op::AddWindow { params },
        Op::Communicate(1),
    ]);
    let r1 = rects(&layout)[&1];

    assert_eq!(
        under(&layout, mid_x(r1), r1.loc.y - 3.),
        Some((1, ResizeEdge::TOP))
    );
    assert_eq!(
        under(&layout, mid_x(r1), bottom(r1) + 3.),
        Some((1, ResizeEdge::BOTTOM))
    );
    assert_eq!(
        under(&layout, r1.loc.x - 3., r1.loc.y - 3.),
        Some((1, ResizeEdge::TOP_LEFT))
    );
}

#[test]
fn zone_drag_resizes_column() {
    let mut layout = layout_with([
        Op::AddOutput(1),
        window(1),
        window(2),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::FocusColumnFirst,
    ]);
    let r1 = rects(&layout)[&1];

    let (id, edges) = under(&layout, right(r1) + 3., mid_y(r1)).unwrap();
    assert_eq!((id, edges), (1, ResizeEdge::RIGHT));

    assert!(layout.interactive_resize_begin(id, edges));
    check_ops_on_layout(
        &mut layout,
        [
            Op::InteractiveResizeUpdate {
                window: 1,
                dx: 50.,
                dy: 0.,
            },
            Op::Communicate(1),
            Op::InteractiveResizeEnd { window: 1 },
            Op::Communicate(1),
            Op::CompleteAnimations,
        ],
    );

    let after = rects(&layout)[&1];
    assert_eq!(after.size.w, r1.size.w + 50.);
}
