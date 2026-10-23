//! Split groups: a static grid of slots inside one column.
//!
//! See `docs/wiki/Development:-Split-Groups.md` for the design. This module is the pure tree: which
//! slots there are, how they split the group's area, and how the tree changes. The column
//! (`scrolling.rs`) owns the tiles; filled slot `i` (depth-first, skipping vacant slots) holds
//! `column.tiles[i]`.

use std::sync::atomic::{AtomicU64, Ordering};

use smithay::utils::{Logical, Point, Rectangle, Size};

static NEXT_GROUP_ID: AtomicU64 = AtomicU64::new(1);

/// How a split arranges its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitKind {
    /// Children next to each other, left to right.
    SideBySide,
    /// Children on top of each other, top to bottom.
    Stacked,
}

/// A side of a slot or of the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A place for one window. Vacant while its window is being dragged.
    Slot { vacant: bool },
    Split {
        kind: SplitKind,
        children: Vec<Child>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    /// Fraction of the split's length (minus gaps) this child gets. Shares of a split add up to 1.
    pub share: f64,
    pub node: Node,
}

/// Path from the root to a node: child indices.
pub type Path = Vec<usize>;

/// Where a drop on a slot edge (or a programmatic insert) puts the new window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeTarget {
    /// Insert as child `index` of the split at `split` (a crevice between two children, or one end).
    Insert { split: Path, index: usize },
    /// Split the slot at `slot` in two, the new window on `side`.
    SplitSlot { slot: Path, side: Side },
    /// The group's outer left or right edge: a new column next to the group.
    NewColumn { right: bool },
}

#[derive(Debug, Clone)]
pub struct Group {
    id: u64,
    root: Node,
    /// Target rectangles of every slot (vacant ones included), depth-first, relative to the
    /// group's top-left corner. Filled by `layout()`.
    rects: Vec<Rectangle<f64, Logical>>,
    /// Target size of the whole group, from the last `layout()`.
    size: Size<f64, Logical>,
}

impl SplitKind {
    pub fn rotated(self) -> Self {
        match self {
            SplitKind::SideBySide => SplitKind::Stacked,
            SplitKind::Stacked => SplitKind::SideBySide,
        }
    }

    /// The split kind for an area: side by side when it's at least as wide as it is tall.
    pub fn for_area(w: f64, h: f64) -> Self {
        if w >= h {
            SplitKind::SideBySide
        } else {
            SplitKind::Stacked
        }
    }

    fn along(self, size: Size<f64, Logical>) -> f64 {
        match self {
            SplitKind::SideBySide => size.w,
            SplitKind::Stacked => size.h,
        }
    }

    fn across(self, size: Size<f64, Logical>) -> f64 {
        match self {
            SplitKind::SideBySide => size.h,
            SplitKind::Stacked => size.w,
        }
    }

    fn size(self, along: f64, across: f64) -> Size<f64, Logical> {
        match self {
            SplitKind::SideBySide => Size::from((along, across)),
            SplitKind::Stacked => Size::from((across, along)),
        }
    }

    fn along_pt(self, p: Point<f64, Logical>) -> f64 {
        match self {
            SplitKind::SideBySide => p.x,
            SplitKind::Stacked => p.y,
        }
    }

    fn across_pt(self, p: Point<f64, Logical>) -> f64 {
        match self {
            SplitKind::SideBySide => p.y,
            SplitKind::Stacked => p.x,
        }
    }

    fn point(self, along: f64, across: f64) -> Point<f64, Logical> {
        match self {
            SplitKind::SideBySide => Point::from((along, across)),
            SplitKind::Stacked => Point::from((across, along)),
        }
    }
}

impl Side {
    /// The split kind whose crevices lie on this side.
    pub fn kind(self) -> SplitKind {
        match self {
            Side::Left | Side::Right => SplitKind::SideBySide,
            Side::Top | Side::Bottom => SplitKind::Stacked,
        }
    }

    /// Whether this is the start side (left or top) along its axis.
    pub fn is_start(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }

    pub fn opposite(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

impl Node {
    fn slot() -> Self {
        Node::Slot { vacant: false }
    }

    fn split_of(kind: SplitKind, nodes: Vec<Node>) -> Self {
        let n = nodes.len() as f64;
        Node::Split {
            kind,
            children: nodes
                .into_iter()
                .map(|node| Child {
                    share: 1. / n,
                    node,
                })
                .collect(),
        }
    }

    fn for_each_slot(&self, path: &mut Path, f: &mut impl FnMut(&Path, bool)) {
        match self {
            Node::Slot { vacant } => f(path, *vacant),
            Node::Split { children, .. } => {
                for (idx, child) in children.iter().enumerate() {
                    path.push(idx);
                    child.node.for_each_slot(path, f);
                    path.pop();
                }
            }
        }
    }

    fn slot_count(&self) -> usize {
        match self {
            Node::Slot { .. } => 1,
            Node::Split { children, .. } => children.iter().map(|c| c.node.slot_count()).sum(),
        }
    }

    fn filled_count(&self) -> usize {
        match self {
            Node::Slot { vacant } => usize::from(!vacant),
            Node::Split { children, .. } => children.iter().map(|c| c.node.filled_count()).sum(),
        }
    }

    /// Minimum size of this subtree. `mins` gives the minimum size of each filled slot in order.
    fn min_size(
        &self,
        gaps: f64,
        mins: &mut impl Iterator<Item = Size<f64, Logical>>,
    ) -> Size<f64, Logical> {
        match self {
            Node::Slot { vacant: true } => Size::from((1., 1.)),
            Node::Slot { vacant: false } => mins.next().unwrap_or(Size::from((1., 1.))),
            Node::Split { kind, children } => {
                let mut along = 0.;
                let mut across: f64 = 0.;
                for child in children {
                    let min = child.node.min_size(gaps, mins);
                    along += kind.along(min);
                    across = across.max(kind.across(min));
                }
                along += gaps * (children.len().saturating_sub(1)) as f64;
                kind.size(along, across)
            }
        }
    }

    /// Lays out this subtree in `rect`, pushing one rectangle per slot in depth-first order.
    fn layout(
        &self,
        rect: Rectangle<f64, Logical>,
        gaps: f64,
        mins: &[Size<f64, Logical>],
        next_min: &mut usize,
        out: &mut Vec<Rectangle<f64, Logical>>,
    ) {
        match self {
            Node::Slot { vacant } => {
                if !vacant {
                    *next_min += 1;
                }
                out.push(rect);
            }
            Node::Split { kind, children } => {
                let kind = *kind;
                // Minimum lengths along the split, per child.
                let mut it = mins[(*next_min).min(mins.len())..].iter().copied();
                let child_mins: Vec<f64> = children
                    .iter()
                    .map(|c| kind.along(c.node.min_size(gaps, &mut it)))
                    .collect();
                let shares: Vec<f64> = children.iter().map(|c| c.share).collect();
                let avail = (kind.along(rect.size) - gaps * (children.len() - 1) as f64).max(0.);
                let lens = distribute(avail, &shares, &child_mins);

                let across = kind.across(rect.size);
                let mut pos = 0.;
                for (child, len) in children.iter().zip(lens) {
                    let loc = rect.loc + kind.point(pos, 0.);
                    let child_rect = Rectangle::new(loc, kind.size(len, across));
                    child.node.layout(child_rect, gaps, mins, next_min, out);
                    pos += len + gaps;
                }
            }
        }
    }

    /// Size this subtree takes with the slots' actual sizes.
    fn extent(
        &self,
        gaps: f64,
        sizes: &mut impl Iterator<Item = Size<f64, Logical>>,
        rects: &mut impl Iterator<Item = Rectangle<f64, Logical>>,
    ) -> Size<f64, Logical> {
        match self {
            Node::Slot { vacant } => {
                let rect = rects.next().unwrap_or_default();
                if *vacant {
                    rect.size
                } else {
                    sizes.next().unwrap_or(rect.size)
                }
            }
            Node::Split { kind, children } => {
                let mut along = 0.;
                let mut across: f64 = 0.;
                for child in children {
                    let size = child.node.extent(gaps, sizes, rects);
                    along += kind.along(size);
                    across = across.max(kind.across(size));
                }
                along += gaps * (children.len().saturating_sub(1)) as f64;
                kind.size(along, across)
            }
        }
    }

    fn get(&self, path: &[usize]) -> Option<&Node> {
        match path.split_first() {
            None => Some(self),
            Some((idx, rest)) => match self {
                Node::Split { children, .. } => children.get(*idx)?.node.get(rest),
                Node::Slot { .. } => None,
            },
        }
    }

    fn get_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        match path.split_first() {
            None => Some(self),
            Some((idx, rest)) => match self {
                Node::Split { children, .. } => children.get_mut(*idx)?.node.get_mut(rest),
                Node::Slot { .. } => None,
            },
        }
    }

    /// Collapses one-child splits and splices same-kind children into their parent, bottom-up.
    ///
    /// Same-kind nesting is only spliced if `flatten` is set: rotation creates it on purpose.
    fn normalize(&mut self, flatten: bool) {
        let Node::Split { kind, children } = self else {
            return;
        };
        let kind = *kind;
        for child in children.iter_mut() {
            child.node.normalize(flatten);
        }
        children
            .retain(|c| !matches!(&c.node, Node::Split { children, .. } if children.is_empty()));

        if flatten {
            let mut flat = Vec::with_capacity(children.len());
            for child in children.drain(..) {
                match child.node {
                    Node::Split {
                        kind: k,
                        children: grand,
                    } if k == kind => {
                        for g in grand {
                            flat.push(Child {
                                share: g.share * child.share,
                                node: g.node,
                            });
                        }
                    }
                    node => flat.push(Child {
                        share: child.share,
                        node,
                    }),
                }
            }
            *children = flat;
        }

        normalize_shares(children);

        if children.len() == 1 {
            let child = children.pop().unwrap();
            *self = child.node;
        }
    }
}

fn normalize_shares(children: &mut [Child]) {
    let total: f64 = children.iter().map(|c| c.share).sum();
    if total > 0. && total.is_finite() {
        for c in children.iter_mut() {
            c.share /= total;
        }
    } else {
        let n = children.len() as f64;
        for c in children.iter_mut() {
            c.share = 1. / n;
        }
    }
}

/// Splits `avail` into integer lengths by `shares`, giving every child at least its minimum
/// (unless the minimums alone don't fit), and adding up to `avail` exactly.
pub fn distribute(avail: f64, shares: &[f64], mins: &[f64]) -> Vec<f64> {
    let n = shares.len();
    let avail = avail.floor().max(0.);
    let mins: Vec<f64> = mins.iter().map(|m| m.ceil().max(1.)).collect();
    let mut fixed: Vec<Option<f64>> = vec![None; n];

    if mins.iter().sum::<f64>() >= avail {
        return mins;
    }

    loop {
        let free_share: f64 = (0..n)
            .filter(|i| fixed[*i].is_none())
            .map(|i| shares[i])
            .sum();
        let free_avail = avail - fixed.iter().flatten().sum::<f64>();
        let mut changed = false;
        for i in 0..n {
            if fixed[i].is_some() {
                continue;
            }
            let len = if free_share > 0. {
                free_avail * shares[i] / free_share
            } else {
                free_avail / (n as f64)
            };
            if len < mins[i] {
                fixed[i] = Some(mins[i]);
                changed = true;
            }
        }
        if !changed || fixed.iter().all(Option::is_some) {
            break;
        }
    }

    let free_share: f64 = (0..n)
        .filter(|i| fixed[*i].is_none())
        .map(|i| shares[i])
        .sum();
    let free_count = fixed.iter().filter(|f| f.is_none()).count() as f64;
    let free_avail = avail - fixed.iter().flatten().sum::<f64>();
    let exact: Vec<f64> = (0..n)
        .map(|i| match fixed[i] {
            Some(len) => len,
            None if free_share > 0. => free_avail * shares[i] / free_share,
            None => free_avail / free_count,
        })
        .collect();

    // Round down, then hand out the leftover pixels by the largest remainders.
    let mut lens: Vec<f64> = exact.iter().map(|l| l.floor()).collect();
    let mut left = (avail - lens.iter().sum::<f64>()).round() as i64;
    let mut order: Vec<usize> = (0..n).filter(|i| fixed[*i].is_none()).collect();
    order.sort_by(|a, b| {
        let ra = exact[*a] - lens[*a];
        let rb = exact[*b] - lens[*b];
        rb.partial_cmp(&ra).unwrap().then(a.cmp(b))
    });
    let mut i = 0;
    while left > 0 && !order.is_empty() {
        lens[order[i % order.len()]] += 1.;
        left -= 1;
        i += 1;
    }
    lens
}

impl Group {
    fn with_root(root: Node) -> Self {
        Self {
            id: NEXT_GROUP_ID.fetch_add(1, Ordering::Relaxed),
            root,
            rects: Vec::new(),
            size: Size::default(),
        }
    }

    /// A group of `count` slots in one split, with the given shares (or equal ones).
    pub fn new(kind: SplitKind, count: usize, shares: Option<&[f64]>) -> Self {
        assert!(count >= 2, "a group needs at least two slots");
        let mut root = Node::split_of(kind, (0..count).map(|_| Node::slot()).collect());
        if let (Some(shares), Node::Split { children, .. }) = (shares, &mut root) {
            for (child, share) in children.iter_mut().zip(shares) {
                child.share = share.max(0.);
            }
            normalize_shares(children);
        }
        Self::with_root(root)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn set_id(&mut self, id: u64) {
        self.id = id;
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Target size of the group from the last layout.
    pub fn size(&self) -> Size<f64, Logical> {
        self.size
    }

    /// Target rectangles of all slots (vacant ones included), depth-first.
    pub fn slot_rects(&self) -> &[Rectangle<f64, Logical>] {
        &self.rects
    }

    /// Target rectangles of the filled slots (tile order).
    pub fn filled_rects(&self) -> Vec<Rectangle<f64, Logical>> {
        self.slots()
            .iter()
            .zip(&self.rects)
            .filter(|((_, vacant), _)| !vacant)
            .map(|(_, r)| *r)
            .collect()
    }

    /// Number of filled slots that come before the subtree at `path` (depth-first).
    pub fn filled_before(&self, path: &[usize]) -> usize {
        self.slots()
            .iter()
            .take_while(|(p, _)| !p.starts_with(path))
            .filter(|(_, vacant)| !vacant)
            .count()
    }

    /// Current lengths of the children of the split at `split` along the split.
    pub fn child_lens(&self, split: &[usize]) -> Vec<f64> {
        let Some(Node::Split { kind, children }) = self.node(split) else {
            return Vec::new();
        };
        (0..children.len())
            .map(|idx| {
                let mut path = split.to_vec();
                path.push(idx);
                self.node_rect(&path).map_or(0., |r| kind.along(r.size))
            })
            .collect()
    }

    /// Minimum lengths of the children of the split at `split` along the split.
    pub fn child_min_lens(
        &self,
        split: &[usize],
        gaps: f64,
        mins: &[Size<f64, Logical>],
    ) -> Vec<f64> {
        let Some(Node::Split { kind, children }) = self.node(split) else {
            return Vec::new();
        };
        let start = self.filled_before(split).min(mins.len());
        let mut it = mins[start..].iter().copied();
        children
            .iter()
            .map(|c| kind.along(c.node.min_size(gaps, &mut it)))
            .collect()
    }

    /// Paths of all slots, depth-first, with whether they are vacant.
    pub fn slots(&self) -> Vec<(Path, bool)> {
        let mut out = Vec::new();
        self.root
            .for_each_slot(&mut Vec::new(), &mut |path, vacant| {
                out.push((path.clone(), vacant))
            });
        out
    }

    pub fn slot_count(&self) -> usize {
        self.root.slot_count()
    }

    pub fn filled_count(&self) -> usize {
        self.root.filled_count()
    }

    pub fn has_vacancy(&self) -> bool {
        self.slot_count() != self.filled_count()
    }

    /// Path of the slot holding tile `tile_idx`.
    pub fn path_of_tile(&self, tile_idx: usize) -> Path {
        self.slots()
            .into_iter()
            .filter(|(_, vacant)| !vacant)
            .nth(tile_idx)
            .map(|(path, _)| path)
            .unwrap_or_else(|| panic!("no slot for tile {tile_idx}"))
    }

    /// Index among all slots (vacant included) of tile `tile_idx`.
    pub fn slot_index_of_tile(&self, tile_idx: usize) -> usize {
        self.slots()
            .into_iter()
            .enumerate()
            .filter(|(_, (_, vacant))| !vacant)
            .nth(tile_idx)
            .map(|(idx, _)| idx)
            .unwrap()
    }

    /// Tile index of the filled slot at `path`, or `None` if it's vacant or not a slot.
    pub fn tile_of_path(&self, path: &[usize]) -> Option<usize> {
        let mut idx = 0;
        for (p, vacant) in self.slots() {
            if p == path {
                return (!vacant).then_some(idx);
            }
            if !vacant {
                idx += 1;
            }
        }
        None
    }

    /// The vacant slot, if any: its path and rectangle.
    pub fn vacancy(&self) -> Option<(Path, Rectangle<f64, Logical>)> {
        self.slots()
            .into_iter()
            .enumerate()
            .find(|(_, (_, vacant))| *vacant)
            .map(|(idx, (path, _))| (path, self.rects.get(idx).copied().unwrap_or_default()))
    }

    pub fn node(&self, path: &[usize]) -> Option<&Node> {
        self.root.get(path)
    }

    /// Kind of the split at `path`.
    pub fn split_kind(&self, path: &[usize]) -> Option<SplitKind> {
        match self.node(path)? {
            Node::Split { kind, .. } => Some(*kind),
            Node::Slot { .. } => None,
        }
    }

    pub fn min_size(&self, gaps: f64, mins: &[Size<f64, Logical>]) -> Size<f64, Logical> {
        self.root.min_size(gaps, &mut mins.iter().copied())
    }

    /// Computes the slot rectangles for a group of `size` (integer-rounded down).
    ///
    /// `mins` are the minimum sizes of the filled slots, in order.
    pub fn layout(&mut self, size: Size<f64, Logical>, gaps: f64, mins: &[Size<f64, Logical>]) {
        let size = Size::from((size.w.floor().max(1.), size.h.floor().max(1.)));
        let mut out = Vec::with_capacity(self.slot_count());
        self.root
            .layout(Rectangle::from_size(size), gaps, mins, &mut 0, &mut out);

        // When the minimum heights don't fit, a split takes its minimum and goes past the bottom.
        // Only the slots whose own minimum needs it may: the others stop at the group's bottom.
        let mut filled = 0;
        for (rect, (_, vacant)) in out.iter_mut().zip(self.slots()) {
            let min_h = if vacant {
                1.
            } else {
                filled += 1;
                mins.get(filled - 1).map_or(1., |m| m.h)
            };
            let room = size.h - rect.loc.y;
            if rect.size.h > room {
                rect.size.h = f64::max(1., f64::max(room, min_h.min(rect.size.h)));
            }
        }

        self.rects = out;
        self.size = size;
    }

    /// Rectangle of the node at `path`: the bounding box of its slots.
    pub fn node_rect(&self, path: &[usize]) -> Option<Rectangle<f64, Logical>> {
        let mut bbox: Option<Rectangle<f64, Logical>> = None;
        for ((p, _), rect) in self.slots().iter().zip(&self.rects) {
            if p.starts_with(path) {
                bbox = Some(match bbox {
                    Some(b) => b.merge(*rect),
                    None => *rect,
                });
            }
        }
        bbox
    }

    /// Size of the group with the tiles' actual sizes (filled slots in order); vacant slots use
    /// their target size.
    pub fn extent(
        &self,
        gaps: f64,
        sizes: impl Iterator<Item = Size<f64, Logical>>,
    ) -> Size<f64, Logical> {
        let mut sizes = sizes;
        let mut rects = self.rects.iter().copied();
        self.root.extent(gaps, &mut sizes, &mut rects)
    }

    /// Path from the root to the slot of `tile_idx`: (kind, index, count, share) per split.
    pub fn path_info(&self, tile_idx: usize) -> Vec<(SplitKind, usize, usize, f64)> {
        let path = self.path_of_tile(tile_idx);
        let mut node = &self.root;
        let mut out = Vec::new();
        for idx in path {
            let Node::Split { kind, children } = node else {
                break;
            };
            out.push((*kind, idx, children.len(), children[idx].share));
            node = &children[idx].node;
        }
        out
    }

    // -- Tree changes ------------------------------------------------------------------------
    //
    // Every change keeps the depth-first order of the slots that stay, so the column only has to
    // insert or remove the tile at the index that the change returns.

    /// Inserts a new slot as child `index` of the split at `split`. The new child gets 1/(n+1);
    /// the others keep their relative shares. Returns the new slot's path.
    pub fn insert_child(&mut self, split: &[usize], index: usize) -> Path {
        let Some(Node::Split { children, .. }) = self.root.get_mut(split) else {
            panic!("insert_child: {split:?} is not a split");
        };
        let n = children.len() as f64;
        for c in children.iter_mut() {
            c.share *= n / (n + 1.);
        }
        let index = index.min(children.len());
        children.insert(
            index,
            Child {
                share: 1. / (n + 1.),
                node: Node::slot(),
            },
        );
        normalize_shares(children);
        let mut path = split.to_vec();
        path.push(index);
        path
    }

    /// Splits the slot at `slot` into two halves, the new slot on `side`. If the slot's parent
    /// already splits that way, the new slot becomes an equal sibling instead. Returns the new
    /// slot's path.
    pub fn split_slot(&mut self, slot: &[usize], side: Side) -> Path {
        let kind = side.kind();
        let (parent, idx) = slot.split_at(slot.len() - 1);
        let idx = idx[0];
        if self.split_kind(parent) == Some(kind) {
            return self.insert_child(parent, if side.is_start() { idx } else { idx + 1 });
        }
        let node = self.root.get_mut(slot).expect("split_slot: no such slot");
        let old = std::mem::replace(node, Node::slot());
        let new_first = side.is_start();
        let nodes = if new_first {
            vec![Node::slot(), old]
        } else {
            vec![old, Node::slot()]
        };
        *node = Node::split_of(kind, nodes);
        let mut path = slot.to_vec();
        path.push(if new_first { 0 } else { 1 });
        path
    }

    /// Puts a new slot next to the whole group on `side`: an equal sibling if the root splits
    /// that way, otherwise a new root with the new slot and the old root, half each.
    pub fn insert_at_group_side(&mut self, side: Side) -> Path {
        let kind = side.kind();
        if self.split_kind(&[]) == Some(kind) {
            let Node::Split { children, .. } = &self.root else {
                unreachable!()
            };
            let index = if side.is_start() { 0 } else { children.len() };
            return self.insert_child(&[], index);
        }
        let old = std::mem::replace(&mut self.root, Node::slot());
        let new_first = side.is_start();
        let nodes = if new_first {
            vec![Node::slot(), old]
        } else {
            vec![old, Node::slot()]
        };
        self.root = Node::split_of(kind, nodes);
        vec![if new_first { 0 } else { 1 }]
    }

    /// Puts a new slot next to the slot at `slot`: an equal sibling if its split has `side`'s
    /// orientation, otherwise splitting the slot in half. Returns the new slot's path.
    pub fn insert_beside(&mut self, slot: &[usize], side: Side) -> Path {
        self.split_slot(slot, side)
    }

    /// Applies an edge target. `NewColumn` is not a tree change and returns `None`.
    pub fn insert_at(&mut self, target: &EdgeTarget) -> Option<Path> {
        match target {
            EdgeTarget::Insert { split, index } => Some(self.insert_child(split, *index)),
            EdgeTarget::SplitSlot { slot, side } => Some(self.split_slot(slot, *side)),
            EdgeTarget::NewColumn { .. } => None,
        }
    }

    /// Removes the slot at `path`; its siblings share its space. Collapses and flattens the tree.
    ///
    /// Returns `false` if the group is left with fewer than two slots (it should dissolve).
    pub fn remove_slot(&mut self, path: &[usize]) -> bool {
        let (parent, idx) = path.split_at(path.len() - 1);
        if let Some(Node::Split { children, .. }) = self.root.get_mut(parent) {
            children.remove(idx[0]);
        }
        self.root.normalize(true);
        matches!(self.root, Node::Split { .. })
    }

    /// Rotates the split at `path` between side by side and stacked. Never flattens.
    pub fn rotate(&mut self, path: &[usize]) {
        if let Some(Node::Split { kind, .. }) = self.root.get_mut(path) {
            *kind = kind.rotated();
        }
    }

    /// Marks the slot at `path` vacant.
    pub fn vacate(&mut self, path: &[usize]) {
        if let Some(Node::Slot { vacant }) = self.root.get_mut(path) {
            *vacant = true;
        }
    }

    /// Fills the vacant slot. Returns its path.
    pub fn fill_vacancy(&mut self) -> Option<Path> {
        let (path, _) = self.vacancy()?;
        if let Some(Node::Slot { vacant }) = self.root.get_mut(&path) {
            *vacant = false;
        }
        Some(path)
    }

    /// Removes the vacant slots. Returns `false` if the group should dissolve.
    pub fn clear_vacancy(&mut self) -> bool {
        while let Some((path, _)) = self.vacancy() {
            if !self.remove_slot(&path) {
                return false;
            }
        }
        matches!(self.root, Node::Split { .. })
    }

    /// Sets the share of every child of the split at `path`.
    pub fn set_shares(&mut self, path: &[usize], shares: &[f64]) {
        if let Some(Node::Split { children, .. }) = self.root.get_mut(path) {
            for (c, s) in children.iter_mut().zip(shares) {
                c.share = s.max(0.0001);
            }
            normalize_shares(children);
        }
    }

    /// Nearest ancestor split of `kind` above the slot of `path`: (split path, child index).
    pub fn nearest_split(&self, path: &[usize], kind: SplitKind) -> Option<(Path, usize)> {
        for depth in (0..path.len()).rev() {
            let anc = &path[..depth];
            if self.split_kind(anc) == Some(kind) {
                return Some((anc.to_vec(), path[depth]));
            }
        }
        None
    }

    /// Where a drop on `side` of the slot at `path` goes.
    ///
    /// Goes up the tree to the nearest split with a crevice on that side. At the group's outer
    /// left/right edge it's a new column; at its outer top/bottom edge, the end of the nearest
    /// stacked split above the slot, or otherwise a split of the slot itself.
    pub fn edge_target(&self, path: &[usize], side: Side) -> EdgeTarget {
        let kind = side.kind();
        let mut boundary = None;
        for depth in (0..path.len()).rev() {
            let anc = &path[..depth];
            let idx = path[depth];
            if let Some(Node::Split { kind: k, children }) = self.node(anc) {
                if *k != kind {
                    continue;
                }
                let n = children.len();
                let interior = if side.is_start() {
                    idx > 0
                } else {
                    idx + 1 < n
                };
                if interior {
                    let index = if side.is_start() { idx } else { idx + 1 };
                    return EdgeTarget::Insert {
                        split: anc.to_vec(),
                        index,
                    };
                }
                if boundary.is_none() {
                    boundary = Some((anc.to_vec(), if side.is_start() { 0 } else { n }));
                }
            }
        }
        match side {
            Side::Left => EdgeTarget::NewColumn { right: false },
            Side::Right => EdgeTarget::NewColumn { right: true },
            Side::Top | Side::Bottom => match boundary {
                Some((split, index)) => EdgeTarget::Insert { split, index },
                None => EdgeTarget::SplitSlot {
                    slot: path.to_vec(),
                    side,
                },
            },
        }
    }

    /// Rectangle of the insert line for an edge target, from the current layout.
    pub fn edge_target_line(
        &self,
        target: &EdgeTarget,
        thickness: f64,
        gaps: f64,
    ) -> Option<Rectangle<f64, Logical>> {
        let (rect, kind, at) = match target {
            EdgeTarget::Insert { split, index } => {
                let Some(Node::Split { kind, children }) = self.node(split) else {
                    return None;
                };
                let rect = self.node_rect(split)?;
                let at = if *index == 0 {
                    kind.along_pt(rect.loc) - gaps / 2.
                } else {
                    let mut prev = split.clone();
                    prev.push((*index - 1).min(children.len() - 1));
                    let prev = self.node_rect(&prev)?;
                    kind.along_pt(prev.loc) + kind.along(prev.size) + gaps / 2.
                };
                (rect, *kind, at)
            }
            EdgeTarget::SplitSlot { slot, side } => {
                let rect = self.node_rect(slot)?;
                let kind = side.kind();
                let start = kind.along_pt(rect.loc);
                let at = if side.is_start() {
                    start - gaps / 2.
                } else {
                    start + kind.along(rect.size) + gaps / 2.
                };
                (rect, kind, at)
            }
            EdgeTarget::NewColumn { .. } => return None,
        };
        let across_start = kind.across_pt(rect.loc);
        let loc = kind.point(at - thickness / 2., across_start);
        Some(Rectangle::new(
            loc,
            kind.size(thickness, kind.across(rect.size)),
        ))
    }
}

/// Picks the slot to go to from `from` in the direction of `side`, among `rects` (by index).
///
/// The nearest slot past that edge that overlaps `from` across the direction; ties go to the one
/// covering `from`'s centre line, then to the one with the most overlap.
pub fn neighbor(rects: &[Rectangle<f64, Logical>], from: usize, side: Side) -> Option<usize> {
    let r = rects[from];
    let center = r.loc + r.size.downscale(2.).to_point();
    let eps = 0.5;
    let mut best: Option<(usize, f64, bool, f64)> = None;
    for (idx, c) in rects.iter().enumerate() {
        if idx == from {
            continue;
        }
        let (dist, overlap, covers) = match side {
            Side::Left | Side::Right => {
                let dist = if side == Side::Left {
                    r.loc.x - (c.loc.x + c.size.w)
                } else {
                    c.loc.x - (r.loc.x + r.size.w)
                };
                let overlap = (r.loc.y + r.size.h).min(c.loc.y + c.size.h) - r.loc.y.max(c.loc.y);
                (
                    dist,
                    overlap,
                    c.loc.y <= center.y && center.y < c.loc.y + c.size.h,
                )
            }
            Side::Top | Side::Bottom => {
                let dist = if side == Side::Top {
                    r.loc.y - (c.loc.y + c.size.h)
                } else {
                    c.loc.y - (r.loc.y + r.size.h)
                };
                let overlap = (r.loc.x + r.size.w).min(c.loc.x + c.size.w) - r.loc.x.max(c.loc.x);
                (
                    dist,
                    overlap,
                    c.loc.x <= center.x && center.x < c.loc.x + c.size.w,
                )
            }
        };
        if dist < -eps || overlap <= eps {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, bd, bc, bo)) => {
                if (dist - bd).abs() > eps {
                    dist < bd
                } else if covers != bc {
                    covers
                } else {
                    overlap > bo
                }
            }
        };
        if better {
            best = Some((idx, dist, covers, overlap));
        }
    }
    best.map(|(idx, ..)| idx)
}

/// The slot on `edge` of the group to enter, nearest to `y` (or `x` for top/bottom).
///
/// `preferred` wins if it is on that edge.
pub fn entry_slot(
    rects: &[Rectangle<f64, Logical>],
    group_size: Size<f64, Logical>,
    edge: Side,
    at: f64,
    preferred: Option<usize>,
) -> Option<usize> {
    let on_edge = |r: &Rectangle<f64, Logical>| match edge {
        Side::Left => r.loc.x <= 0.5,
        Side::Right => r.loc.x + r.size.w >= group_size.w - 0.5,
        Side::Top => r.loc.y <= 0.5,
        Side::Bottom => r.loc.y + r.size.h >= group_size.h - 0.5,
    };
    if let Some(p) = preferred {
        if rects.get(p).is_some_and(on_edge) {
            return Some(p);
        }
    }
    rects
        .iter()
        .enumerate()
        .filter(|(_, r)| on_edge(r))
        .min_by(|(_, a), (_, b)| {
            let da = dist_1d(at, a, edge);
            let db = dist_1d(at, b, edge);
            da.partial_cmp(&db).unwrap()
        })
        .map(|(idx, _)| idx)
}

fn dist_1d(at: f64, r: &Rectangle<f64, Logical>, edge: Side) -> f64 {
    let (start, len) = match edge {
        Side::Left | Side::Right => (r.loc.y, r.size.h),
        Side::Top | Side::Bottom => (r.loc.x, r.size.w),
    };
    if at < start {
        start - at
    } else if at >= start + len {
        at - (start + len) + 0.001
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(g: &Group) -> String {
        fn rec(n: &Node, out: &mut String) {
            match n {
                Node::Slot { vacant } => out.push(if *vacant { 'v' } else { 's' }),
                Node::Split { kind, children } => {
                    out.push(if *kind == SplitKind::SideBySide {
                        '['
                    } else {
                        '('
                    });
                    for (i, c) in children.iter().enumerate() {
                        if i > 0 {
                            out.push(' ');
                        }
                        rec(&c.node, out);
                    }
                    out.push(if *kind == SplitKind::SideBySide {
                        ']'
                    } else {
                        ')'
                    });
                }
            }
        }
        let mut s = String::new();
        rec(g.root(), &mut s);
        s
    }

    fn shares(g: &Group, path: &[usize]) -> Vec<f64> {
        match g.node(path).unwrap() {
            Node::Split { children, .. } => children
                .iter()
                .map(|c| (c.share * 1000.).round() / 1000.)
                .collect(),
            _ => panic!(),
        }
    }

    #[test]
    fn distribute_rounds_exactly() {
        assert_eq!(distribute(100., &[0.5, 0.5], &[1., 1.]), vec![50., 50.]);
        assert_eq!(
            distribute(100., &[1. / 3., 1. / 3., 1. / 3.], &[1., 1., 1.]),
            vec![34., 33., 33.]
        );
        assert_eq!(distribute(100., &[0.5, 0.5], &[80., 1.]), vec![80., 20.]);
        assert_eq!(distribute(100., &[0.5, 0.5], &[80., 80.]), vec![80., 80.]);
    }

    #[test]
    fn insert_same_kind_is_equal_sibling() {
        let mut g = Group::new(SplitKind::SideBySide, 2, None);
        assert_eq!(g.insert_at_group_side(Side::Left), vec![0]);
        assert_eq!(kinds(&g), "[s s s]");
        assert_eq!(shares(&g, &[]), vec![0.333, 0.333, 0.333]);

        // Other orientation wraps the root.
        assert_eq!(g.insert_at_group_side(Side::Bottom), vec![1]);
        assert_eq!(kinds(&g), "([s s s] s)");
    }

    #[test]
    fn split_slot_and_flatten_on_remove() {
        let mut g = Group::new(SplitKind::SideBySide, 2, None);
        let p = g.split_slot(&[1], Side::Bottom);
        assert_eq!(p, vec![1, 1]);
        assert_eq!(kinds(&g), "[s (s s)]");
        assert_eq!(g.tile_of_path(&[1, 1]), Some(2));

        // Rotating the inner split keeps the nesting (so it can be rotated back).
        g.rotate(&[1]);
        assert_eq!(kinds(&g), "[s [s s]]");
        g.rotate(&[1]);
        assert_eq!(kinds(&g), "[s (s s)]");

        // Removing the first slot collapses the root into the inner split.
        assert!(g.remove_slot(&[0]));
        assert_eq!(kinds(&g), "(s s)");

        // Rotated nesting flattens on the next structural change.
        let mut g = Group::new(SplitKind::SideBySide, 3, None);
        g.split_slot(&[2], Side::Bottom);
        g.rotate(&[2]);
        assert_eq!(kinds(&g), "[s s [s s]]");
        assert!(g.remove_slot(&[0]));
        assert_eq!(kinds(&g), "[s s s]");
        assert_eq!(shares(&g, &[]), vec![0.5, 0.25, 0.25]);

        assert!(!Group::new(SplitKind::Stacked, 2, None).remove_slot(&[0]));
    }

    #[test]
    fn edge_targets() {
        // [A (B C)]
        let mut g = Group::new(SplitKind::SideBySide, 2, None);
        g.split_slot(&[1], Side::Bottom);
        // B's left edge: the root crevice between A and (B C).
        assert_eq!(
            g.edge_target(&[1, 0], Side::Left),
            EdgeTarget::Insert {
                split: vec![],
                index: 1
            }
        );
        // B's bottom edge: between B and C.
        assert_eq!(
            g.edge_target(&[1, 0], Side::Bottom),
            EdgeTarget::Insert {
                split: vec![1],
                index: 1
            }
        );
        // B's top edge: the top of the stacked split.
        assert_eq!(
            g.edge_target(&[1, 0], Side::Top),
            EdgeTarget::Insert {
                split: vec![1],
                index: 0
            }
        );
        // A's top edge: no stacked split above A, so A's slot splits.
        assert_eq!(
            g.edge_target(&[0], Side::Top),
            EdgeTarget::SplitSlot {
                slot: vec![0],
                side: Side::Top
            }
        );
        // Outer left/right: new columns.
        assert_eq!(
            g.edge_target(&[0], Side::Left),
            EdgeTarget::NewColumn { right: false }
        );
        assert_eq!(
            g.edge_target(&[1, 1], Side::Right),
            EdgeTarget::NewColumn { right: true }
        );
    }

    #[test]
    fn layout_and_neighbors() {
        // [A (B C)] in 410 x 300 with 10 px gaps.
        let mut g = Group::new(SplitKind::SideBySide, 2, None);
        g.split_slot(&[1], Side::Bottom);
        g.layout(Size::from((410., 300.)), 10., &[Size::from((1., 1.)); 3]);
        let r = g.slot_rects().to_vec();
        assert_eq!(
            r[0],
            Rectangle::new(Point::from((0., 0.)), Size::from((200., 300.)))
        );
        assert_eq!(
            r[1],
            Rectangle::new(Point::from((210., 0.)), Size::from((200., 145.)))
        );
        assert_eq!(
            r[2],
            Rectangle::new(Point::from((210., 155.)), Size::from((200., 145.)))
        );

        assert_eq!(neighbor(&r, 0, Side::Right), Some(1));
        assert_eq!(neighbor(&r, 2, Side::Left), Some(0));
        assert_eq!(neighbor(&r, 1, Side::Bottom), Some(2));
        assert_eq!(neighbor(&r, 1, Side::Top), None);
        assert_eq!(neighbor(&r, 0, Side::Left), None);

        assert_eq!(entry_slot(&r, g.size(), Side::Right, 200., None), Some(2));
        assert_eq!(
            entry_slot(&r, g.size(), Side::Right, 200., Some(1)),
            Some(1)
        );
        assert_eq!(entry_slot(&r, g.size(), Side::Left, 10., None), Some(0));

        assert_eq!(
            g.extent(
                10.,
                r.iter().map(|r| r.size).collect::<Vec<_>>().into_iter()
            ),
            g.size()
        );

        // Min sizes push the split.
        g.layout(
            Size::from((410., 300.)),
            10.,
            &[
                Size::from((300., 1.)),
                Size::from((1., 1.)),
                Size::from((1., 1.)),
            ],
        );
        assert_eq!(g.slot_rects()[0].size.w, 300.);
        assert_eq!(g.slot_rects()[1].size.w, 100.);
    }

    #[test]
    fn vacancy() {
        let mut g = Group::new(SplitKind::SideBySide, 3, None);
        g.vacate(&[1]);
        assert!(g.has_vacancy());
        assert_eq!(g.filled_count(), 2);
        assert_eq!(g.tile_of_path(&[2]), Some(1));
        assert_eq!(g.path_of_tile(1), vec![2]);
        assert!(g.clear_vacancy());
        assert_eq!(kinds(&g), "[s s]");

        let mut g = Group::new(SplitKind::SideBySide, 2, None);
        g.vacate(&[0]);
        assert_eq!(g.fill_vacancy(), Some(vec![0]));
        g.vacate(&[0]);
        assert!(!g.clear_vacancy());
    }
}
