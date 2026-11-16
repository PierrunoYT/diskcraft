//! Squarified treemap layout and painting.

use eframe::egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, ecolor::Hsva, pos2, vec2,
};

use crate::format;
use crate::tree::{NodeId, Tree};

/// Directories nest at most this many levels below the focus.
const MAX_DEPTH: u8 = 8;
/// Smallest frame side that still gets its children laid out inside it.
const MIN_NEST: f32 = 24.0;
const HEADER: f32 = 15.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// A directory whose children are drawn inside it.
    Frame,
    /// A file, or a directory too small to open up.
    Leaf,
}

pub struct Cell {
    pub id: NodeId,
    pub rect: Rect,
    pub kind: CellKind,
    color: Color32,
}

/// Laid-out treemap for one focus directory and one canvas rectangle.
pub struct Treemap {
    pub focus: NodeId,
    pub rect: Rect,
    /// Parents precede their descendants, so later cells are drawn on top.
    pub cells: Vec<Cell>,
}

impl Treemap {
    pub fn new(tree: &Tree, focus: NodeId, rect: Rect) -> Self {
        let mut cells = Vec::new();
        layout_dir(tree, focus, rect, 0, None, &mut cells);
        Self { focus, rect, cells }
    }

    /// Innermost cell under `pos`.
    pub fn hit(&self, pos: Pos2) -> Option<&Cell> {
        self.cells.iter().rev().find(|c| c.rect.contains(pos))
    }

    /// Cell showing `id`, or its nearest drawn ancestor.
    pub fn cell_for(&self, tree: &Tree, id: NodeId) -> Option<&Cell> {
        tree.ancestors_inclusive(id)
            .take_while(|&a| a != self.focus)
            .find_map(|a| self.cells.iter().find(|c| c.id == a))
    }

    pub fn paint(&self, painter: &Painter, tree: &Tree, selected: Option<NodeId>, hovered: Option<NodeId>) {
        let border = Stroke::new(1.0, Color32::from_black_alpha(90));
        let font = FontId::proportional(11.0);
        for cell in &self.cells {
            let r = cell.rect;
            painter.rect_filled(r, 0.0, cell.color);
            if r.width() >= 3.0 && r.height() >= 3.0 {
                painter.rect_stroke(r, 0.0, border, StrokeKind::Inside);
            }

            let node = tree.node(cell.id);
            let text_color = if cell.kind == CellKind::Frame { Color32::WHITE } else { Color32::BLACK };
            let label = match cell.kind {
                CellKind::Frame if r.width() > 40.0 && r.height() > 48.0 => {
                    Some(format!("{}  {}", tree.name(cell.id), format::size(node.allocated)))
                }
                CellKind::Leaf if r.width() > 48.0 && r.height() > 16.0 => Some(tree.name(cell.id).to_owned()),
                _ => None,
            };
            if let Some(label) = label {
                painter.with_clip_rect(r.shrink(1.0)).text(
                    r.min + vec2(4.0, 1.0),
                    Align2::LEFT_TOP,
                    label,
                    font.clone(),
                    text_color,
                );
            }
        }

        let outline = |id: Option<NodeId>, stroke: Stroke| {
            if let Some(cell) = id.and_then(|id| self.cell_for(tree, id)) {
                painter.rect_stroke(cell.rect, 0.0, stroke, StrokeKind::Inside);
            }
        };
        outline(hovered, Stroke::new(1.5, Color32::from_rgb(255, 230, 120)));
        outline(selected, Stroke::new(2.5, Color32::WHITE));
    }
}

fn layout_dir(tree: &Tree, dir: NodeId, rect: Rect, depth: u8, hue: Option<f32>, out: &mut Vec<Cell>) {
    // The tree view may have re-sorted children; the layout needs them by disk usage.
    // Areas use allocated size: online-only placeholders report their full length but use no disk.
    let mut items: Vec<(NodeId, u64)> = tree
        .children(dir)
        .iter()
        .map(|&c| (c, tree.node(c).allocated))
        .filter(|&(_, size)| size > 0)
        .collect();
    items.sort_unstable_by_key(|&(_, size)| std::cmp::Reverse(size));
    let weights: Vec<f64> = items.iter().map(|&(_, s)| s as f64).collect();

    for (k, (&(id, _), r)) in items.iter().zip(squarify(&weights, rect)).enumerate() {
        if r.width() * r.height() < 2.0 {
            continue;
        }
        // Each top-level branch gets its own hue, spread by the golden ratio.
        let hue = hue.unwrap_or_else(|| (k as f32 * 0.618_034 + 0.08).fract());
        let node = tree.node(id);
        let nest = node.is_dir && depth < MAX_DEPTH && r.width() > MIN_NEST && r.height() > MIN_NEST;
        if nest {
            let value = (0.42 - 0.04 * depth as f32).max(0.18);
            out.push(Cell { id, rect: r, kind: CellKind::Frame, color: Hsva::new(hue, 0.45, value, 1.0).into() });
            let header = if r.width() > 40.0 && r.height() > 48.0 { HEADER } else { 0.0 };
            let inner = Rect::from_min_max(r.min + vec2(2.0, 2.0 + header), r.max - vec2(2.0, 2.0));
            layout_dir(tree, id, inner, depth + 1, Some(hue), out);
        } else {
            let (sat, value) = if node.is_dir { (0.30, 0.70) } else { (0.50, 0.92 - 0.04 * depth as f32) };
            out.push(Cell { id, rect: r, kind: CellKind::Leaf, color: Hsva::new(hue, sat, value.max(0.55), 1.0).into() });
        }
    }
}

/// Squarified treemap (Bruls, Huizing, van Wijk). `weights` must be positive and sorted
/// descending; returns one rectangle per weight, with areas proportional to the weights.
pub fn squarify(weights: &[f64], rect: Rect) -> Vec<Rect> {
    let total: f64 = weights.iter().sum();
    let (mut x, mut y) = (rect.min.x as f64, rect.min.y as f64);
    let (mut w, mut h) = (rect.width() as f64, rect.height() as f64);
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return vec![Rect::NOTHING; weights.len()];
    }
    let scale = w * h / total;
    let mut out = Vec::with_capacity(weights.len());

    let mut i = 0;
    while i < weights.len() {
        let side = w.min(h);
        let first = weights[i] * scale;
        let (mut sum, mut min, mut max) = (first, first, first);
        let mut worst = worst_aspect(sum, min, max, side);
        let mut j = i + 1;
        while j < weights.len() {
            let a = weights[j] * scale;
            let candidate = worst_aspect(sum + a, min.min(a), max.max(a), side);
            if candidate > worst {
                break;
            }
            (sum, min, max, worst) = (sum + a, min.min(a), max.max(a), candidate);
            j += 1;
        }

        if w >= h {
            // Column along the left edge.
            let strip = if h > 0.0 { sum / h } else { 0.0 };
            let mut cy = y;
            for &wt in &weights[i..j] {
                let ch = if strip > 0.0 { wt * scale / strip } else { 0.0 };
                out.push(rect_f64(x, cy, strip, ch));
                cy += ch;
            }
            x += strip;
            w = (w - strip).max(0.0);
        } else {
            // Row along the top edge.
            let strip = if w > 0.0 { sum / w } else { 0.0 };
            let mut cx = x;
            for &wt in &weights[i..j] {
                let cw = if strip > 0.0 { wt * scale / strip } else { 0.0 };
                out.push(rect_f64(cx, y, cw, strip));
                cx += cw;
            }
            y += strip;
            h = (h - strip).max(0.0);
        }
        i = j;
    }
    out
}

/// Worst aspect ratio of a strip holding areas summing to `sum` along a side of length `side`.
fn worst_aspect(sum: f64, min: f64, max: f64, side: f64) -> f64 {
    let (s2, side2) = (sum * sum, side * side);
    (side2 * max / s2).max(s2 / (side2 * min))
}

fn rect_f64(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::from_min_max(pos2(x as f32, y as f32), pos2((x + w) as f32, (y + h) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_tiles_rect_proportionally() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 400.0));
        let weights = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&weights, rect);
        assert_eq!(rects.len(), weights.len());

        let total: f64 = weights.iter().sum();
        let area = |r: &Rect| (r.width() * r.height()) as f64;
        for (r, w) in rects.iter().zip(weights) {
            let expected = w / total * 600.0 * 400.0;
            assert!((area(r) - expected).abs() < 1.0, "{r:?} should have area {expected}");
            assert!(rect.expand(0.01).contains_rect(*r), "{r:?} escapes {rect:?}");
        }
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let overlap = a.intersect(*b);
                assert!(overlap.width() < 0.01 || overlap.height() < 0.01, "{a:?} overlaps {b:?}");
            }
        }
    }
}
