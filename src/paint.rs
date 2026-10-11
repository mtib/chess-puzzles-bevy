//! The board painter — a CPU twin of the QML Canvas in the Omarchy panel:
//! black board, diagonal-hatched light squares, dashed cell outlines, accent
//! marks for moves, and arrows (auto move arrows, plus the user's
//! shift+drag arrows).

use crate::model::{square_at, Board};
use crate::pieces::PieceSprites;
use crate::state::PanelState;
use crate::theme::{self, with_alpha, Rgba};

const HATCH: Rgba = theme::LINE_STRONG;
/// Opaque, so the pixel where two outlines cross isn't blended twice.
const GRID: Rgba = theme::GRID;
const MOVE_FILL: Rgba = with_alpha(theme::ACCENT, 0.40);
const WRONG_FILL: Rgba = with_alpha(theme::ACCENT, 0.85);
const MOVE_ARROW: Rgba = theme::ACCENT;
const USER_ARROW: Rgba = with_alpha(theme::UPCOMING, 0.85);
const USER_ARROW_PREVIEW: Rgba = with_alpha(theme::UPCOMING, 0.60);

/// Fraction of a cell a piece fills.
const PIECE_SCALE: f64 = 0.92;
/// Fade steps of the auto arrow: repaints during a fade are capped to these.
const ALPHA_STEPS: f32 = 32.0;

/// Everything the board picture depends on. Comparing two views tells
/// whether a repaint is needed.
#[derive(Clone, PartialEq)]
pub struct BoardView {
    pub board: Board,
    pub flipped: bool,
    pub selected: Option<usize>,
    pub wrong: Option<usize>,
    pub last_move: Option<(usize, usize)>,
    pub user_arrows: Vec<(usize, usize)>,
    pub drag_arrow: Option<(usize, usize)>,
    /// Auto move arrow with its alpha quantized to `ALPHA_STEPS`.
    pub auto_arrow: Option<(usize, usize, u8)>,
}

impl BoardView {
    pub fn of(state: &PanelState) -> Self {
        BoardView {
            board: state.board,
            flipped: state.is_flipped(),
            selected: state.selected,
            wrong: state.wrong_flash.map(|(square, _)| square),
            last_move: state.last_move,
            user_arrows: state.user_arrows.clone(),
            drag_arrow: state.drag_arrow.filter(|(from, to)| from != to),
            auto_arrow: state
                .auto_arrow
                .map(|a| (a.from, a.to, (a.alpha * ALPHA_STEPS).round() as u8))
                .filter(|a| a.2 > 0),
        }
    }
}

/// Texel size of the pieces for a given cell size.
pub fn piece_size(cell: u32) -> u32 {
    (cell as f64 * PIECE_SCALE).round() as u32
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

    fn blend(&mut self, x: i64, y: i64, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let sa = c[3];
        let da = self.data[i + 3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        for (k, s) in c.iter().take(3).enumerate() {
            let d = self.data[i + k] as f32 / 255.0;
            let mixed = if out_a > 0.0 { (s * sa + d * da * (1.0 - sa)) / out_a } else { 0.0 };
            self.data[i + k] = (mixed.min(1.0) * 255.0).round() as u8;
        }
        self.data[i + 3] = (out_a.min(1.0) * 255.0).round() as u8;
    }

    fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgba) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend(xx, yy, c);
            }
        }
    }

    /// A `width`-texel ring just inside the rect.
    fn ring(&mut self, x: i64, y: i64, size: i64, width: i64, c: Rgba) {
        self.fill_rect(x, y, size, width, c);
        self.fill_rect(x, y + size - width, size, width, c);
        self.fill_rect(x, y + width, width, size - 2 * width, c);
        self.fill_rect(x + size - width, y + width, width, size - 2 * width, c);
    }

    fn line(&mut self, from: (f64, f64), to: (f64, f64), width: f64, c: Rgba) {
        let len = (to.0 - from.0).hypot(to.1 - from.1);
        if len <= 0.0 {
            return;
        }
        let steps = (len * 2.0).ceil() as i64;
        let w = width / 2.0;
        let woff = w.ceil() as i64;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let cx = from.0 + (to.0 - from.0) * t;
            let cy = from.1 + (to.1 - from.1) * t;
            for oy in -woff..=woff {
                for ox in -woff..=woff {
                    if (ox as f64).hypot(oy as f64) <= w {
                        self.blend(cx as i64 + ox, cy as i64 + oy, c);
                    }
                }
            }
        }
    }

    /// A dashed line exactly one texel wide, `len` texels long, along x
    /// (`vertical` false) or y. Integer dash and gap lengths, so every dash
    /// is the same size.
    fn dashes(&mut self, x: i64, y: i64, len: i64, vertical: bool, dash: (i64, i64), c: Rgba) {
        let (on, off) = dash;
        for i in (0..len).filter(|i| i % (on + off) < on) {
            if vertical {
                self.blend(x, y + i, c);
            } else {
                self.blend(x + i, y, c);
            }
        }
    }

    /// Copies a sprite 1:1 at an integer position — no resampling.
    fn blit(&mut self, img: &image::RgbaImage, x: i64, y: i64) {
        for (sx, sy, p) in img.enumerate_pixels() {
            if p[3] > 0 {
                let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0];
                self.blend(x + sx as i64, y + sy as i64, c);
            }
        }
    }
}

/// Paints the board texture: 8 x `cell` texels square, 1:1 with the screen.
/// `ring` is the selection ring width in texels; `pieces` must be rasterized
/// at `piece_size(cell)`.
pub fn paint_board(view: &BoardView, pieces: &PieceSprites, cell: u32, ring: u32) -> Vec<u8> {
    let tex = cell * 8;
    let c = cell as i64;
    let mut px = Px::new(tex, tex);
    px.fill_rect(0, 0, tex as i64, tex as i64, theme::BG);

    // Top-left texel of a board index on screen.
    let pos_of = |idx: usize| -> (i64, i64) {
        let screen = square_at(idx / 8, idx % 8, view.flipped);
        ((screen % 8) as i64 * c, (screen / 8) as i64 * c)
    };

    // Square tints under the hatch: the wrong attempt, then the last move.
    for i in 0..64usize {
        let tint = if view.wrong == Some(i) {
            Some(WRONG_FILL)
        } else if view.last_move.is_some_and(|(from, to)| i == from || i == to) {
            Some(MOVE_FILL)
        } else {
            None
        };
        if let Some(tint) = tint {
            let (x, y) = pos_of(i);
            px.fill_rect(x, y, c, c, tint);
        }
    }

    // Diagonal hatching on the light squares: pixel-exact 45° stripes along
    // x+y = const — no line-drawing aliasing.
    let spacing = (cell * 5 / 54).max(2);
    for row in 0..8u32 {
        for file in 0..8u32 {
            if (row + file) % 2 != 0 {
                continue;
            }
            let (x0, y0) = (file * cell, row * cell);
            for yy in y0..y0 + cell {
                for xx in x0..x0 + cell {
                    if (xx + yy).is_multiple_of(spacing) {
                        px.blend(xx as i64, yy as i64, HATCH);
                    }
                }
            }
        }
    }

    // Dashed cell outlines, 1 texel wide. The pattern restarts at every
    // square so all edges look alike. Outline g sits on the first texel row
    // or column of square g; the last one on the board's final texel.
    let dash = (((cell * 5) / 54).max(2) as i64, ((cell * 4) / 54).max(1) as i64);
    for g in 0..=8i64 {
        let p = (g * c).min(tex as i64 - 1);
        for k in 0..8i64 {
            px.dashes(p, k * c, c, true, dash, GRID);
            px.dashes(k * c, p, c, false, dash, GRID);
        }
    }

    if let Some(sel) = view.selected {
        let (x, y) = pos_of(sel);
        px.ring(x, y, c, ring as i64, theme::ACCENT);
    }

    // Arrows: user-drawn, the live preview, then the automatic move arrow.
    let cell_f = cell as f64;
    let draw_arrow = |px: &mut Px, from: usize, to: usize, color: Rgba| {
        let half = cell_f / 2.0;
        let (fx, fy) = pos_of(from);
        let (tx, ty) = pos_of(to);
        let (fx, fy) = (fx as f64 + half, fy as f64 + half);
        let (dx, dy) = (tx as f64 + half - fx, ty as f64 + half - fy);
        let len = dx.hypot(dy);
        if len <= 1.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let head = (cell_f * 0.3).max(6.0);
        let tip = (fx + dx - ux * cell_f * 0.34, fy + dy - uy * cell_f * 0.34);
        let shaft = (tip.0 - ux * head, tip.1 - uy * head);
        let width = (cell_f * 0.09).max(2.0);
        px.line((fx, fy), shaft, width, color);
        let p2 = (shaft.0 - uy * head * 0.45, shaft.1 + ux * head * 0.45);
        let p3 = (shaft.0 + uy * head * 0.45, shaft.1 - ux * head * 0.45);
        let centroid = ((tip.0 + p2.0 + p3.0) / 3.0, (tip.1 + p2.1 + p3.1) / 3.0);
        px.line(centroid, p2, width, color);
        px.line(centroid, p3, width, color);
        px.line(p2, p3, width, color);
    };
    for &(from, to) in &view.user_arrows {
        draw_arrow(&mut px, from, to, USER_ARROW);
    }
    if let Some((from, to)) = view.drag_arrow {
        draw_arrow(&mut px, from, to, USER_ARROW_PREVIEW);
    }
    if let Some((from, to, alpha)) = view.auto_arrow {
        draw_arrow(&mut px, from, to, with_alpha(MOVE_ARROW, alpha as f32 / ALPHA_STEPS));
    }

    // Pieces last, centred on whole texels.
    let inset = (c - pieces.size as i64) / 2;
    for (i, square) in view.board.iter().enumerate() {
        if let Some(img) = square.and_then(|piece| pieces.get(piece)) {
            let (x, y) = pos_of(i);
            px.blit(img, x + inset, y + inset);
        }
    }

    px.data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pieces::PieceArt;

    fn view() -> BoardView {
        let mut state = PanelState::with_settings(1500, 0);
        let (board, turn) = crate::model::parse_fen("6k1/2Q1R3/3p2p1/2p3N1/2bbP3/5P2/r5P1/4K3 b - - 1 1");
        state.board = board;
        state.user_color = turn;
        BoardView::of(&state)
    }

    #[test]
    fn hatch_light_squares() {
        let pieces = PieceArt::load(std::path::Path::new("/nonexistent")).rasterize(50);
        let data = paint_board(&view(), &pieces, 54, 2);
        let [r, g, b, _] = HATCH.map(|v| (v * 255.0).round() as u8);
        let hatch = data.chunks(4).filter(|p| p[0] == r && p[1] == g && p[2] == b).count();
        assert!(hatch > 1000, "expected thousands of hatch pixels, got {hatch}");
    }

    #[test]
    fn outlines_are_exactly_one_texel_wide() {
        let empty = BoardView { board: [None; 64], ..view() };
        let pieces = PieceArt::load(std::path::Path::new("/nonexistent")).rasterize(1);
        let grid = GRID.map(|v| (v * 255.0).round() as u8);
        for cell in [22u32, 37, 54, 59, 95, 153] {
            let tex = cell * 8;
            let data = paint_board(&empty, &pieces, cell, 2);
            let is_grid = |x: u32, y: u32| {
                let i = ((y * tex + x) * 4) as usize;
                data[i..i + 3] == grid[..3]
            };
            let column = |x: u32| (0..tex).filter(|&y| is_grid(x, y)).count();
            let lines: Vec<u32> = (0..=8).map(|g| (g * cell).min(tex - 1)).collect();
            for x in 0..tex {
                let n = column(x);
                if lines.contains(&x) {
                    assert!(n as u32 > tex / 3, "cell {cell}: outline at x={x} too sparse ({n})");
                } else {
                    // Only the crossings of horizontal outlines.
                    assert!(n <= 9, "cell {cell}: stray outline texels at x={x} ({n})");
                }
            }
        }
    }

    #[test]
    fn pieces_rasterize_at_requested_size() {
        let art = PieceArt::load(&crate::paths::assets_dir().join("pieces"));
        let sprites = art.rasterize(piece_size(54));
        let view = view();
        let piece = view.board.iter().flatten().next().copied().unwrap();
        let img = sprites.get(piece).expect("piece svg should load");
        assert_eq!(img.dimensions(), (50, 50));
        assert!(img.pixels().any(|p| p[3] == 255), "rendered piece has opaque pixels");
    }
}
