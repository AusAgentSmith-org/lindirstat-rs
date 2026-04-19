//! Squarified treemap layout (Bruls, Huijsing, Van Wijk 2000).

use egui::Rect;

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub index: usize,
    pub rect: Rect,
    pub depth: u16,
    pub is_leaf: bool,
}

/// Squarified layout of one level. Returns `(item_index, rect)` pairs.
pub fn squarified(mut items: Vec<(usize, u64)>, area: Rect) -> Vec<(usize, Rect)> {
    items.retain(|(_, s)| *s > 0);
    items.sort_by(|a, b| b.1.cmp(&a.1));
    if items.is_empty() {
        return vec![];
    }
    let total: u128 = items.iter().map(|(_, s)| *s as u128).sum();
    if total == 0 {
        return vec![];
    }
    let scale = (area.width() as f64 * area.height() as f64) / total as f64;
    let scaled: Vec<(usize, f64)> = items
        .into_iter()
        .map(|(i, s)| (i, s as f64 * scale))
        .collect();

    let mut out = Vec::new();
    let mut remaining = area;
    let mut i = 0;
    while i < scaled.len() {
        let w = short_side(remaining);
        let mut row_end = i;
        while row_end < scaled.len() {
            let next_end = row_end + 1;
            if row_end == i || worst(&scaled[i..next_end], w) <= worst(&scaled[i..row_end], w) {
                row_end = next_end;
            } else {
                break;
            }
        }
        remaining = lay_row(&scaled[i..row_end], remaining, &mut out);
        i = row_end;
    }
    out
}

fn short_side(r: Rect) -> f64 {
    (r.width().min(r.height())) as f64
}

fn worst(row: &[(usize, f64)], w: f64) -> f64 {
    if row.is_empty() {
        return f64::INFINITY;
    }
    let mut s = 0.0;
    let mut rmin = f64::INFINITY;
    let mut rmax: f64 = 0.0;
    for (_, v) in row {
        s += *v;
        if *v < rmin {
            rmin = *v;
        }
        if *v > rmax {
            rmax = *v;
        }
    }
    let w2 = w * w;
    let s2 = s * s;
    f64::max(w2 * rmax / s2, s2 / (w2 * rmin))
}

fn lay_row(row: &[(usize, f64)], rect: Rect, out: &mut Vec<(usize, Rect)>) -> Rect {
    let s: f64 = row.iter().map(|(_, v)| v).sum();
    if s == 0.0 {
        return rect;
    }
    if rect.width() <= rect.height() {
        // Horizontal strip across the top.
        let strip_h = (s / rect.width() as f64) as f32;
        let strip_h = strip_h.min(rect.height());
        let mut x = rect.min.x;
        for (idx, v) in row {
            let item_w = (*v / s * rect.width() as f64) as f32;
            let r = Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(item_w, strip_h));
            out.push((*idx, r));
            x += item_w;
        }
        Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + strip_h), rect.max)
    } else {
        // Vertical strip down the left.
        let strip_w = (s / rect.height() as f64) as f32;
        let strip_w = strip_w.min(rect.width());
        let mut y = rect.min.y;
        for (idx, v) in row {
            let item_h = (*v / s * rect.height() as f64) as f32;
            let r = Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(strip_w, item_h));
            out.push((*idx, r));
            y += item_h;
        }
        Rect::from_min_max(egui::pos2(rect.min.x + strip_w, rect.min.y), rect.max)
    }
}
