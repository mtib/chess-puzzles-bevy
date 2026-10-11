//! Piece art: the caliente SVGs, rasterized with resvg at the exact texel size
//! the board paints them at. Pieces are blitted 1:1, never resampled, so they
//! stay sharp at every cell size and display scale.

use crate::model::{Color, Kind, Piece};
use image::RgbaImage;
use resvg::{tiny_skia, usvg};
use std::path::Path;

const KINDS: [(Kind, char); 6] = [
    (Kind::Pawn, 'p'),
    (Kind::Knight, 'n'),
    (Kind::Bishop, 'b'),
    (Kind::Rook, 'r'),
    (Kind::Queen, 'q'),
    (Kind::King, 'k'),
];

fn slot(piece: Piece) -> usize {
    let kind = KINDS.iter().position(|(k, _)| *k == piece.kind).unwrap_or(0);
    kind * 2 + (piece.color == Color::Black) as usize
}

/// The parsed vector art, one tree per piece (missing files stay `None`).
pub struct PieceArt {
    trees: Vec<Option<usvg::Tree>>,
}

impl PieceArt {
    /// Loads `{kind}{w|b}.svg` from `dir`, e.g. `nw.svg` for the white knight.
    pub fn load(dir: &Path) -> Self {
        let options = usvg::Options::default();
        let mut trees = Vec::with_capacity(12);
        for (_, kind) in KINDS {
            for color in ['w', 'b'] {
                let path = dir.join(format!("{kind}{color}.svg"));
                let tree = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|data| usvg::Tree::from_data(&data, &options).map_err(|e| e.to_string()));
                match tree {
                    Ok(tree) => trees.push(Some(tree)),
                    Err(err) => {
                        bevy::log::warn!("piece art {}: {err}", path.display());
                        trees.push(None);
                    }
                }
            }
        }
        PieceArt { trees }
    }

    /// Rasterizes every piece into a `size` x `size` square.
    pub fn rasterize(&self, size: u32) -> PieceSprites {
        let images = self.trees.iter().map(|tree| tree.as_ref().and_then(|t| render(t, size))).collect();
        PieceSprites { size, images }
    }
}

/// The pieces at one texel size, straight (non-premultiplied) RGBA.
pub struct PieceSprites {
    pub size: u32,
    images: Vec<Option<RgbaImage>>,
}

impl PieceSprites {
    pub fn get(&self, piece: Piece) -> Option<&RgbaImage> {
        self.images.get(slot(piece)).and_then(Option::as_ref)
    }
}

fn render(tree: &usvg::Tree, size: u32) -> Option<RgbaImage> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = size as f32 / w.max(h);
    let dx = (size as f32 - w * scale) / 2.0;
    let dy = (size as f32 - h * scale) / 2.0;
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    RgbaImage::from_raw(size, size, rgba)
}
