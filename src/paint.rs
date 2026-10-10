//! The board painter — a CPU twin of the QML Canvas in the Omarchy panel:
//! monochrome dark background, orange squares for moves, diagonal-hatched
//! light squares, dashed cell outlines, and arrows (auto move arrows, plus
//! the user's shift+drag arrows).

use crate::model::{Color, Kind};
use crate::state::PanelState;
use image::RgbaImage;

pub const TEX: u32 = 512; // texture size: 8 cells of 64px
pub const CELL: u32 = TEX / 8;

pub const BG: [f32; 4] = [0x0e as f32 / 255.0, 0x0e as f32 / 255.0, 0x10 as f32 / 255.0, 1.0];
pub const HATCH: [f32; 4] = [0x3c as f32 / 255.0, 0x3d as f32 / 255.0, 0x44 as f32 / 255.0, 1.0];
pub const GRID: [f32; 4] = [0x45 as f32 / 255.0, 0x46 as f32 / 255.0, 0x4e as f32 / 255.0, 1.0];
pub const MOVE: [f32; 4] = [0xe8 as f32 / 255.0, 0x89 as f32 / 255.0, 0x0c as f32 / 255.0, 1.0];
pub const MOVE_FILL: [f32; 4] = [232.0 / 255.0, 138.0 / 255.0, 12.0 / 255.0, 0.42];
pub const WRONG: [f32; 4] = [201.0 / 255.0, 79.0 / 255.0, 61.0 / 255.0, 0.55];
pub const SELECTED: [f32; 4] = [232.0 / 255.0, 138.0 / 255.0, 12.0 / 255.0, 0.30];
pub const USER_ARROW: [f32; 4] = [0xe6 as f32 / 255.0, 0xe6 as f32 / 255.0, 0xe6 as f32 / 255.0, 0.85];
pub const USER_ARROW_PREVIEW: [f32; 4] = [0xe6 as f32 / 255.0, 0xe6 as f32 / 255.0, 0xe6 as f32 / 255.0, 0.60];

/// A pixel canvas: RGBA8, float colors, alpha blending.
struct Px {
    data: Vec<u8>,
    w: u32,
    h: u32,
}

impl Px {
    fn new(w: u32, h: u32) -> Self {
        Px { data: vec![0; (w * h * 4) as usize], w, h }
    }

    fn blend(&mut self, x: i64, y: i64, c: [f32; 4]) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let sa = c[3];
        let da = self.data[i + 3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        for k in 0..3 {
            let s = c[k];
            let d = self.data[i + k] as f32 / 255.0;
            let mixed = if out_a > 0.0 {
                (s * sa + d * da * (1.0 - sa)) / out_a
            } else {
                0.0
            };
            self.data[i + k] = (mixed.min(1.0) * 255.0).round() as u8;
        }
        self.data[i + 3] = (out_a.min(1.0) * 255.0).round() as u8;
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: [f32; 4]) {
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let x1 = (x + w).ceil() as i64;
        let y1 = (y + h).ceil() as i64;
        for yy in y0..y1 {
            for xx in x0..x1 {
                self.blend(xx, yy, c);
            }
        }
    }

    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, width: f64, c: [f32; 4]) {
        let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if len <= 0.0 {
            return;
        }
        let steps = (len * 2.0).ceil() as i64;
        let w = width / 2.0;
        let woff = (w.ceil()) as i64;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let cx = x0 + (x1 - x0) * t;
            let cy = y0 + (y1 - y0) * t;
            for oy in -woff..=woff {
                for ox in -woff..=woff {
                    if (ox as f64).hypot(oy as f64) <= w {
                        self.blend(cx as i64 + ox, cy as i64 + oy, c);
                    }
                }
            }
        }
    }

    /// A dashed line, mirroring ctx.setLineDash([on, off]).
    fn dashed_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, on: f64, off: f64, c: [f32; 4]) {
        let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if len <= 0.0 {
            return;
        }
        let ux = (x1 - x0) / len;
        let uy = (y1 - y0) / len;
        let period = on + off;
        let mut d = 0.0;
        while d < len {
            let seg = period.min(len - d);
            let draw = seg.min(on);
            self.line(x0 + ux * d, y0 + uy * d, x0 + ux * (d + draw), y0 + uy * (d + draw), 1.0, c);
            d += seg;
        }
    }

    fn blit(&mut self, img: &RgbaImage, dx: f64, dy: f64, dw: f64, dh: f64) {
        // Bilinear sampling of the source into the destination rect.
        for yy in 0..dh.ceil() as u32 {
            for xx in 0..dw.ceil() as u32 {
                let sx = (xx as f64 / dw * img.width() as f64).min(img.width() as f64 - 1.0);
                let sy = (yy as f64 / dh * img.height() as f64).min(img.height() as f64 - 1.0);
                let sx0 = sx.floor() as u32;
                let sy0 = sy.floor() as u32;
                let fx = sx - sx0 as f64;
                let fy = sy - sy0 as f64;
                let mut acc = [0.0f32; 4];
                for (ox, oy, wgt) in
                    [(0u32, 0u32, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
                {
                    let sxn = (sx0 + ox).min(img.width() - 1);
                    let syn = (sy0 + oy).min(img.height() - 1);
                    let p = img.get_pixel(sxn, syn);
                    for k in 0..4 {
                        acc[k] += p[k] as f32 / 255.0 * wgt as f32;
                    }
                }
                if acc[3] <= 0.0 {
                    continue;
                }
                self.blend(
                    (dx + xx as f64) as i64,
                    (dy + yy as f64) as i64,
                    [acc[0], acc[1], acc[2], acc[3]],
                );
            }
        }
    }
}

/// Paint the full board texture for the current state.
pub fn paint_board(state: &PanelState, pieces: &std::collections::HashMap<String, RgbaImage>) -> Vec<u8> {
    let mut px = Px::new(TEX, TEX);
    px.fill_rect(0.0, 0.0, TEX as f64, TEX as f64, BG);

    let cell = CELL as f64;

    // Screen position of a board index, honoring orientation: the side to
    // move (or the flipped view) sits at the bottom.
    let flip = state.user_color == Color::Black;
    let pos_of = |idx: usize| -> (f64, f64) {
        let row = idx / 8;
        let file = idx % 8;
        let (r, f) = if flip { (7 - row, 7 - file) } else { (row, file) };
        (f as f64 * cell, r as f64 * cell)
    };
    let _pos_center = |idx: usize| -> (f64, f64) {
        let (x, y) = pos_of(idx);
        (x + cell / 2.0, y + cell / 2.0)
    };

    // Involved squares first: selection, wrong attempt, and the last move's
    // from/to, all orange.
    for i in 0..64usize {
        let tint = if state.wrong_flash == Some(i) {
            Some(WRONG)
        } else if state.selected == Some(i) {
            Some(SELECTED)
        } else if state.last_from == Some(i) || state.last_to == Some(i) {
            Some(MOVE_FILL)
        } else {
            None
        };
        if let Some(c) = tint {
            let (x, y) = pos_of(i);
            px.fill_rect(x, y, cell, cell, c);
        }
    }

    // Diagonal hatching on the light squares: clipped 45° lines.
    for row in 0..8 {
        for file in 0..8 {
            if (row + file) % 2 != 0 {
                continue;
            }
            let x = file as f64 * cell;
            let y = row as f64 * cell;
            let step = cell / 9.0; // mirrors the QML 5px hatch at a 53px cell
            let mut d = -cell;
            while d < cell + cell {
                // clip: only draw the segment inside the cell
                let sx = x + d;
                let ex = x + d + cell;
                let sy = y + cell;
                let ey = y;
                // param clip to rect
                let clip = |x0: f64, y0: f64, x1: f64, y1: f64| -> Option<(f64, f64, f64, f64)> {
                    // line from (x0,y0) to (x1,y1); clip to rect x..x+cell, y..y+cell
                    let dx = x1 - x0;
                    let dy = y1 - y0;
                    let mut t0 = 0.0f64;
                    let mut t1 = 1.0f64;
                    let edges = [
                        (x - x0, dx, 0.0),
                        (x + cell - x0, dx, 1.0),
                        (y - y0, dy, 0.0),
                        (y + cell - y0, dy, 1.0),
                    ];
                    for (dist, dir, side) in edges {
                        if dir.abs() < 1e-9 {
                            continue;
                        }
                        let t = dist / dir;
                        if (dir > 0.0) == (side > 0.5) {
                            t0 = t0.max(t);
                        } else {
                            t1 = t1.min(t);
                        }
                    }
                    if t0 >= t1 {
                        return None;
                    }
                    Some((x0 + dx * t0, y0 + dy * t0, x0 + dx * t1, y0 + dy * t1))
                };
                if let Some((a, b, c, d2)) = clip(sx, sy, ex, ey) {
                    px.line(a, b, c, d2, 1.0, HATCH);
                }
                d += step;
            }
        }
    }

    // Dashed cell outlines over everything.
    for g in 0..=8 {
        let p = g as f64 * cell;
        px.dashed_line(p, 0.0, p, TEX as f64, 4.8, 3.6, GRID);
        px.dashed_line(0.0, p, TEX as f64, p, 4.8, 3.6, GRID);
    }

    // Arrows: user-drawn (shift+drag), the live preview, then the automatic
    // move arrow in orange.
    let mut draw_arrow = |px: &mut Px, from: usize, to: usize, color: [f32; 4]| {
        let (fx, fy) = pos_of(from);
        let (tx, ty) = pos_of(to);
        let fx = fx + cell / 2.0;
        let fy = fy + cell / 2.0;
        let mut tx = tx + cell / 2.0;
        let mut ty = ty + cell / 2.0;
        let dx = tx - fx;
        let dy = ty - fy;
        let len = dx.hypot(dy);
        if len <= 1.0 {
            return;
        }
        let ux = dx / len;
        let uy = dy / len;
        let head = (cell * 0.3).max(6.0);
        tx -= ux * cell * 0.34;
        ty -= uy * cell * 0.34;
        let shaft_x = tx - ux * head;
        let shaft_y = ty - uy * head;
        let width = (cell * 0.09).max(2.0);
        px.line(fx, fy, shaft_x, shaft_y, width, color);
        // arrowhead triangle
        let p1 = (tx, ty);
        let p2 = (shaft_x - uy * head * 0.45, shaft_y + ux * head * 0.45);
        let p3 = (shaft_x + uy * head * 0.45, shaft_y - ux * head * 0.45);
        let centroid = ((p1.0 + p2.0 + p3.0) / 3.0, (p1.1 + p2.1 + p3.1) / 3.0);
        for (ax, ay) in [p2, p3] {
            px.line(centroid.0, centroid.1, ax, ay, width, color);
        }
        px.line(p2.0, p2.1, p3.0, p3.1, width, color);
    };

    for arrow in &state.user_arrows {
        draw_arrow(&mut px, arrow.0, arrow.1, USER_ARROW);
    }
    if let Some((from, to)) = state.drag_arrow.filter(|(f, t)| f != t) {
        draw_arrow(&mut px, from, to, USER_ARROW_PREVIEW);
    }
    if let Some(arrow) = &state.auto_arrow {
        if arrow.alpha > 0.01 {
            let c = [MOVE[0], MOVE[1], MOVE[2], MOVE[3] * arrow.alpha];
            draw_arrow(&mut px, arrow.from, arrow.to, c);
        }
    }

    // Pieces last, on top of everything.
    for i in 0..64 {
        let Some(piece) = state.board[i] else { continue };
        let (x, y) = pos_of(i);
        let name = format!(
            "{}{}",
            match piece.kind {
                Kind::Pawn => 'p',
                Kind::Knight => 'n',
                Kind::Bishop => 'b',
                Kind::Rook => 'r',
                Kind::Queen => 'q',
                Kind::King => 'k',
            },
            match piece.color {
                Color::White => 'w',
                Color::Black => 'b',
            }
        );
        if let Some(img) = pieces.get(&name) {
            let size = cell * 0.92;
            px.blit(img, x + (cell - size) / 2.0, y + (cell - size) / 2.0, size, size);
        }
    }

    px.data
}

