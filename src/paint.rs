//! The board painter — a CPU twin of the QML Canvas in the Omarchy panel:
//! monochrome dark background, orange squares for moves, diagonal-hatched
//! light squares, dashed cell outlines, and arrows (auto move arrows, plus
//! the user's shift+drag arrows).

use crate::model::{Color, Kind};
use crate::state::PanelState;
use image::RgbaImage;


// Terminal / neo-brutalist palette: pitch black base, coral primary accent.
pub const BG: [f32; 4] = hex("#000000", 1.0); // pure black
pub const HATCH: [f32; 4] = hex("#2a2a30", 1.0); // slate grey
pub const GRID: [f32; 4] = hex("#3a3a42", 1.0); // subtle grey outline
pub const MOVE: [f32; 4] = hex("#ff5232", 1.0); // coral
pub const MOVE_FILL: [f32; 4] = [255.0 / 255.0, 82.0 / 255.0, 50.0 / 255.0, 0.40];
pub const WRONG: [f32; 4] = [214.0 / 255.0, 69.0 / 255.0, 69.0 / 255.0, 0.55];
pub const SELECTED: [f32; 4] = [255.0 / 255.0, 82.0 / 255.0, 50.0 / 255.0, 0.28];
pub const USER_ARROW: [f32; 4] = hex("#c9cdd6", 0.85); // monochrome line art
pub const USER_ARROW_PREVIEW: [f32; 4] = hex("#c9cdd6", 0.60);

const fn nib(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'f' => (c - b'a' + 10) as u32,
        _ => 0,
    }
}

const fn hex(hex: &str, a: f32) -> [f32; 4] {
    let b = hex.as_bytes();
    let n = (nib(b[1]) << 20) | (nib(b[2]) << 16) | (nib(b[3]) << 12) | (nib(b[4]) << 8)
        | (nib(b[5]) << 4) | nib(b[6]);
    [
        ((n >> 16) & 0xff) as f32 / 255.0,
        ((n >> 8) & 0xff) as f32 / 255.0,
        (n & 0xff) as f32 / 255.0,
        a,
    ]
}

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
        // Area-averaged (box filter) sampling: each destination pixel averages
        // the source pixels it covers. Crisp minification at any size, without
        // the blur of bilinear downsampling.
        let sw = img.width() as f64;
        let sh = img.height() as f64;
        let x_ratio = sw / dw;
        let y_ratio = sh / dh;
        for yy in 0..dw.ceil().max(dh.ceil()) as u32 {
            if yy >= dh.ceil() as u32 {
                break;
            }
            for xx in 0..dw.ceil() as u32 {
                let sx0 = xx as f64 * x_ratio;
                let sy0 = yy as f64 * y_ratio;
                let sx1 = (sx0 + x_ratio).ceil().min(sw);
                let sy1 = (sy0 + y_ratio).ceil().min(sh);
                let mut acc = [0.0f64; 4];
                let mut n = 0.0;
                let mut sy = sy0.floor();
                while (sy as f64) < sy1 {
                    let mut sx = sx0.floor();
                    while (sx as f64) < sx1 {
                        let p = img.get_pixel((sx as u32).min(img.width() - 1), (sy as u32).min(img.height() - 1));
                        for k in 0..4 {
                            acc[k] += p[k] as f64 / 255.0;
                        }
                        n += 1.0;
                        sx += 1.0;
                    }
                    sy += 1.0;
                }
                if n > 0.0 && acc[3] / n > 0.0 {
                    self.blend(
                        (dx + xx as f64) as i64,
                        (dy + yy as f64) as i64,
                        [(acc[0] / n) as f32, (acc[1] / n) as f32, (acc[2] / n) as f32, (acc[3] / n) as f32],
                    );
                }
            }
        }
    }
}

/// Paint the full board texture for the current state.
/// Paint the full board texture for the current state at a given cell size
/// (the texture is 8x cell, 1:1 with the on-screen board).
pub fn paint_board(
    state: &PanelState,
    pieces: &std::collections::HashMap<String, RgbaImage>,
    cell: u32,
) -> Vec<u8> {
    let tex = cell * 8;
    let cell = cell as f64;
    let mut px = Px::new(tex, tex);
    px.fill_rect(0.0, 0.0, tex as f64, tex as f64, BG);

    
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

    // Diagonal hatching on the light squares: pixel-exact 45° stripes along
    // x+y = const, one pixel every 5 — no line-drawing aliasing.
    for row in 0..8 {
        for file in 0..8 {
            if (row + file) % 2 != 0 {
                continue;
            }
            let x0 = file as u32 * cell as u32;
            let y0 = row as u32 * cell as u32;
            let spacing = ((cell as u32 * 5) / 54).max(2);
            for yy in 0..cell as u32 {
                for xx in 0..cell as u32 {
                    if ((x0 + xx + y0 + yy) % spacing) == 0 {
                        px.blend((x0 + xx) as i64, (y0 + yy) as i64, HATCH);
                    }
                }
            }
        }
    }

    // Dashed cell outlines over everything.
    let tex_f = tex as f64;
    let dash_on = cell * 4.8 / 54.0;
    let dash_off = cell * 3.6 / 54.0;
    for g in 0..=8 {
        // Clamp to tex-1: a line drawn exactly at the edge is clipped out.
        let p = (g as f64 * cell).min(tex_f - 1.0);
        px.dashed_line(p, 0.0, p, tex_f, dash_on, dash_off, GRID);
        px.dashed_line(0.0, p, tex_f, p, dash_on, dash_off, GRID);
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PanelState;

    fn png_bytes(data: &[u8]) -> Vec<u8> {
        // minimal: count pixel colors instead of encoding
        data.to_vec()
    }

    #[test]
    fn hatch_light_squares() {
        let mut state = PanelState::new();
        let (board, turn) = crate::model::parse_fen("6k1/2Q1R3/3p2p1/2p3N1/2bbP3/5P2/r5P1/4K3 b - - 1 1");
        state.board = board;
        state.user_color = turn;
        state.phase = crate::state::Phase::Play;
        let pieces = std::collections::HashMap::new();
        let data = png_bytes(&paint_board(&state, &pieces, 54));
        // count hatch-colored pixels (#3c3d44)
        let mut hatch = 0;
        for px in data.chunks(4) {
            if px[0] == 0x2a && px[1] == 0x2a && px[2] == 0x30 {
                hatch += 1;
            }
        }
        assert!(hatch > 1000, "expected thousands of hatch pixels, got {hatch}");
    }
}
