//! Colour tokens (omarchy "vantablack" style): pure black, barely-cool zinc
//! neutrals, and one accent. Every colour in the app comes from here.

use bevy::color::Color;

/// Straight-alpha RGBA in 0..1, the painter's native format.
pub type Rgba = [f32; 4];

const fn rgb(n: u32) -> Rgba {
    [
        ((n >> 16) & 0xff) as f32 / 255.0,
        ((n >> 8) & 0xff) as f32 / 255.0,
        (n & 0xff) as f32 / 255.0,
        1.0,
    ]
}

pub const fn with_alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], a]
}

pub const BG: Rgba = rgb(0x000000);
pub const CONTROL: Rgba = rgb(0x0b0b0c); // secondary button fill
pub const LINE_STRONG: Rgba = rgb(0x26262a); // control borders, meter tracks
pub const TEXT: Rgba = rgb(0xe8e8e8);
pub const HEADING: Rgba = rgb(0xf2f2f2);
pub const MUTED: Rgba = rgb(0x76767d); // labels, meta
pub const FAINT: Rgba = rgb(0x4a4a50); // disabled
pub const GRID: Rgba = rgb(0x343438); // board outlines (faint at 70% on black)
pub const INVERT: Rgba = rgb(0xe4e4e4); // hover fill, with black text
pub const ACCENT: Rgba = rgb(0xff5a36); // the only colour
pub const ACCENT_HOVER: Rgba = rgb(0xff7d5a);
pub const UPCOMING: Rgba = rgb(0xc9c9cd); // light neutral for user arrows

pub fn ui(c: Rgba) -> Color {
    Color::srgba(c[0], c[1], c[2], c[3])
}
