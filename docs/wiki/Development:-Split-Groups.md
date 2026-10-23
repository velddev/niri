## Split groups: design note

A *split group* puts several windows into one column of the scrolling strip, arranged as a static grid of
slots: side by side, stacked, or nested combinations of both.
For the strip, a group is just a column: it is full height, it scrolls, it has a column width, and every
column action (width presets, `set-column-width`, `maximize-column`, `expand-column-to-available-width`,
moving the column to another workspace or monitor) acts on the whole group.
Inside, the slots are the windows' places: windows are put into slots and can be swapped around, while the
slots stay where they are.

This page is the design note for the fork's implementation (`src/layout/group.rs`, the `group` field of
`Column` in `src/layout/scrolling.rs`).

### Model

A group is a tree owned by the column (`Column::group: Option<Group>`):

```
Group { id, root: Node }
Node  = Slot(Filled | Vacant)
      | Split { kind: SideBySide | Stacked, children: [(share, Node)] }
```

- The root is always a `Split` with at least two children (one may be vacant, see dragging).
  Shares of a split's children add up to 1.
- Slots are numbered depth-first ("reading order").
  Filled slot *i* holds `column.tiles[i]`, so swapping two windows is swapping two tiles; the tree does not
  change.
  A `Vacant` slot exists only while its window is being dragged: it keeps the window's place (and its
  space) so the window can be dropped back, or swapped with another one.
- A column is either a group or a plain column (normal or tabbed); a group is never tabbed, fullscreen or
  maximized.
  When a group drops to one window it dissolves back into a plain column with the group's width.
- Group ids are unique for the session (a counter, like window ids) and are reported over IPC.

Built on niri's column code where it helps: tiles, sizes, transactions, focus ring, the column's width
logic, and all column-level actions are reused unchanged; only the column's internal layout (where its
tiles go and what size they get) switches to the tree when `group` is set.

### Split rule and flattening

`split-window` (Mod+S):

- On a window that is alone in its column, the window merges into the column to its right (at the end of
  the strip: the one to its left).
  It takes half of that column's area along the area's longer side: a wide column (width ≥ height) splits
  side by side, a tall one stacked.
  The window goes on the side it came from (first when it came from the left, last from the right).
  The group keeps the target column's width; the merged window's old column closes.
- If the target is already a group whose root split has that orientation, the window becomes an equal
  sibling instead of nesting: ⅓ ⅓ ⅓, not ½ ¼ ¼.
  In general, a new sibling in a split of *n* children gets 1/(n+1) and the others keep their relative
  sizes (scaled by n/(n+1)), so equal splits stay equal.
  Otherwise the root is wrapped: a new root of the other orientation with the window and the old root,
  ½ each.
- A plain stacked column (niri's consume/expel stacking) involved in a split is first turned into a
  stacked group with the same windows and heights (as shares).
- On a window in a group, `split-window` rotates the split the window is in (its parent split) between
  side by side and stacked.
  Rotating never flattens, so pressing Mod+S twice always gets back to where you were.
- On a window in a plain stacked column, `split-window` turns the stack into a group and rotates it: the
  windows go side by side.
- Tabbed columns are never merged or rotated (the action does nothing), and a lone window with no
  neighbour has nothing to merge with.

Flattening happens on removal: when a split is left with one child, the child takes its place, and a child
split with the same orientation as its new parent is spliced into the parent (shares multiplied).
Inserting into a split never creates a same-orientation nesting, because insertion points always belong to
the split of that orientation.

### Resizing

Two layers, never mixed:

- **The group as a column.** Its width is the column width: `switch-preset-column-width` (Mod+R),
  `set-column-width` (Mod+Minus/Equal), `maximize-column`, `expand-column-to-available-width`, Mod+G
  (`pond-grow`, which uses `set-column-width` with a fixed size: for a group that is the whole group's width)
  and an interactive or edge resize on the group's *outer* left/right edges.
  Its height is always the working area's height.
  Inner shares stay as they are, so the slots scale with the group.
- **Inside the group.** Only the shares change, the group's size never does:
  - an interactive resize (Mod+right-drag, or a drag in the gap: the edge-resize zones) on an edge that
    is a crevice between two slots moves that divider; the two sides grow and shrink together, the rest
    keeps its size;
  - `set-window-width` / `switch-preset-window-width` change the window's share in its nearest
    side-by-side split, `set-window-height` / `switch-preset-window-height` its share in the nearest
    stacked split; `reset-window-height` makes the nearest stacked split's shares equal again.
    With no split of that orientation above the window they do nothing (a group is always full height).
- The group's outer top and bottom edges have no resize zone.
  Windows' min sizes are respected: a split never gives a child less than the child's minimum, and the
  group's minimum width (sum or max of its slots' minimums) is the column's minimum width.

### Dragging: targets and hit zones

Interactive move (Mod+left-drag and title-bar drags) works as in niri, with these targets over a group
(computed from the final, not the animated, geometry):

- Over a slot, the outer band along each edge is an *edge zone*: a quarter of the slot's size, at least
  24 px (or the gap, if larger) and at most 100 px.
  The middle of the slot is the *swap zone*.
  Over the gaps between slots, the nearest slot edge counts.
- **Swap zone of another window: swap.**
  The dragged window takes that slot; the other window goes to where the dragged window came from (its
  vacant slot, its old stacked column, or a new column where its old column was).
  Feedback: the target slot is highlighted (the insert-hint colour over the whole slot).
- **The dragged window's own vacant slot: back to its place** (highlighted the same way).
- **Edge zones** go up the tree to the nearest split that has a gap there:
  - a crevice between two children of a split (of any depth) with the edge's orientation: **insert the
    window there as a new slot** with an equal share;
  - the group's outer left or right edge: **a new column** next to the group, i.e. the window leaves
    the group (niri's normal column insert hint);
  - the group's outer top or bottom edge: the window goes to that end of the nearest stacked split above
    the slot, or, without one, splits the slot itself (½ each).
  Feedback for inserts: an *insert line*, a bar about twice the gap thick along the crevice, spanning
  exactly the split it inserts into.
- Plain (non-group) columns keep niri's behaviour: the nearest gap between columns or between stacked
  windows.
  Windows in plain columns are not swap targets.
- While a window from a group is being dragged, its slot stays as a vacant slot.
  On drop the vacancy is resolved (filled or removed; a group left with one window dissolves).
  `Layout::refresh()` clears any vacancy left by a drag that ended without a drop (window closed
  mid-drag, output removed), so a vacancy never outlives the drag.

### Focus and move rules

- Directional focus moves between the slots of the group first, geometrically: the nearest slot in that
  direction that overlaps the current one, preferring the one under the current slot's centre line.
  From the edge of the group it continues as usual: `focus-column-left/right` to the neighbouring
  column, `focus-window-or-workspace-up/down` to the neighbouring workspace, `focus-window-up/down`
  stops, `focus-window-up-or-column-left` etc. go to the column.
- Entering a group from the side picks the slot on that edge nearest to the previously focused window's
  vertical centre (the group's remembered window if it is on that edge).
- `consume-or-expel-window-left/right` (Mod+Ctrl+Left/Right) inside a group swaps with the neighbouring
  slot; at the group's left/right edge the window leaves the group into its own column on that side.
  From a lone column next to a group it merges into the group, by the split rule.
- `move-window-up/down` and `move-window-up/down-or-to-workspace-up/down` swap with the slot above or
  below; at the group's edge the first do nothing and the second move the window to the workspace.
  `swap-window-left/right` swaps slots, and at the group's edge swaps with the neighbouring column's
  window.
- `unsplit-window` (Mod+Shift+S) pops the window out into its own column next to the group: on the left
  if its slot touches only the group's left edge, otherwise on the right.
  The new column keeps the window's width (the group's width setting if the slot spans the whole group).
  On a window in a plain stacked column it expels the window the same way.
- `move-column-*`, `move-column-to-workspace-*` and `move-column-to-monitor-*` move the whole group;
  `move-window-to-workspace-*`, `move-window-to-monitor-*` and `toggle-window-floating` take the window
  out of its group.
- **Fullscreen and maximize** are per window: the window leaves its group into its own column right next
  to it (as niri already does for a window in a stacked column) and goes fullscreen on its own.
  It remembers its place; when it leaves fullscreen or maximize it goes back into its slot: exactly
  (same tree, same shares) if the group is unchanged, otherwise next to its old neighbour, otherwise it
  stays a column.
- Closing a window removes its slot (siblings share its space by their shares); a group of one
  dissolves.

### Animations

Everything uses niri's existing animation config: structural changes (merge, rotate, swap, insert, pop
out, dissolve) animate tiles from their old positions with `window-movement`, size changes with
`window-resize` (the tile resize animation plus a matching move of the slot's position), the view with
`horizontal-view-movement`.
Nothing waits for an animation.
The layout state changes immediately and animations only render the difference: hit testing, focus,
drop targets and IPC all use the final geometry, and a new change mid-animation starts from where the
tile is drawn now (niri's move animations add the current offset), so reversing, e.g. Mod+S twice
quickly, is instant and continuous.

### IPC

All additions are optional fields, so existing clients keep working.

- `Window.layout.group` (in `niri msg windows`, `focused-window` and the event stream's
  `WindowsChanged`/`WindowOpenedOrChanged`/`WindowLayoutsChanged`): `null` outside groups, otherwise
  `{ group_id, slot, slot_count, path, pos_in_group, size_in_group, group_size }`.
  `slot` is 1-based in reading order and equals the second number of `pos_in_scrolling_layout`.
  `path` lists the splits from the root down to the slot:
  `[{ split: "SideBySide" | "Stacked", index, count, share }]` (`index` 0-based), enough to rebuild the
  whole tree from the windows of a group.
  Positions and sizes are the slot's target geometry in logical pixels relative to the group's top-left.
- `WindowRect.group` (`niri msg window-rects`): `null`, or `{ group_id, pos, size }`, the group's
  rectangle on the output as drawn, like `pos`/`size`.
- Actions: `split-window [--id]`, `unsplit-window [--id]`, and
  `split-window-beside --target ID --side left|right|up|down [--id]`, which puts a window next to another
  one as a new slot (an equal sibling if the target's split has that orientation, otherwise splitting the
  target's slot in half), for tools that rebuild layouts (both windows on the same workspace).
  Shares can then be restored with `set-window-width`/`set-window-height --id` (inner resizing).
- Clients reading the event stream through `niri msg -j event-stream` need the new `niri msg`:
  an older one parses the events into its own types and drops the fields it doesn't know.

### Hard limits and deviations

- Nested splits are kept even when they end up with the same orientation as their parent after a
  rotation (otherwise Mod+S twice couldn't undo itself); they only flatten when the tree changes
  structurally.
- Fullscreen and maximize take the window out of the group while they last (a group is one column, and a
  column is fullscreen as a whole in niri), but the window comes back to its slot afterwards.
- Windows in plain columns are not swap targets, so niri's drop-into-a-stack keeps working.
