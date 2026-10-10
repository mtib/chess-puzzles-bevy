//! Chess model — mirrors the QML panel's Model.js exactly, including its
//! deliberate simplification: there is no move generator, the puzzle's
//! solution list is the only judge of legality.

use serde::Deserialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    White,
    Black,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Piece {
    pub color: Color,
    pub kind: Kind,
}

/// 64-entry board in FEN order: index 0 is a8, index 63 is h1.
pub type Board = [Option<Piece>; 64];

/// "e4" -> 36 (rank 8 rows at the top, a8 = 0)
pub fn square_to_index(square: &str) -> usize {
    let file = square.as_bytes()[0] - b'a';
    let rank = square.as_bytes()[1] - b'0';
    ((8 - rank) * 8 + file) as usize
}

pub fn index_to_square(index: usize) -> String {
    let file = b'a' + (index % 8) as u8;
    let rank = b'0' + (8 - index / 8) as u8;
    String::from_utf8(vec![file, rank]).unwrap()
}

fn parse_piece(ch: u8) -> Piece {
    let color = if ch.is_ascii_uppercase() { Color::White } else { Color::Black };
    let kind = match ch.to_ascii_lowercase() {
        b'p' => Kind::Pawn,
        b'n' => Kind::Knight,
        b'b' => Kind::Bishop,
        b'r' => Kind::Rook,
        b'q' => Kind::Queen,
        _ => Kind::King,
    };
    Piece { color, kind }
}

/// "6k1/2Q1R3/... b - - 1 1" -> (board, side to move)
pub fn parse_fen(fen: &str) -> (Board, Color) {
    let mut board: Board = [None; 64];
    let mut parts = fen.split_whitespace();
    let rows = parts.next().unwrap_or("");
    let turn = match parts.next() {
        Some("b") => Color::Black,
        _ => Color::White,
    };
    for (r, row) in rows.split('/').take(8).enumerate() {
        let mut file = 0usize;
        for ch in row.bytes() {
            if ch.is_ascii_digit() {
                file += (ch - b'0') as usize;
            } else {
                board[r * 8 + file] = Some(parse_piece(ch));
                file += 1;
            }
        }
    }
    (board, turn)
}

/// Play a UCI move ("e2e4", "e7e8q") onto a board copy. Handles the three
/// moves that touch more than one square: castling moves the rook, en passant
/// removes the passed pawn, promotion swaps the piece.
pub fn apply_uci(board: &Board, uci: &str) -> Board {
    let mut next = *board;
    let from = square_to_index(&uci[0..2]);
    let to = square_to_index(&uci[2..4]);
    let piece = match board[from] {
        Some(p) => p,
        None => return next,
    };
    next[to] = Some(piece);
    next[from] = None;

    // En passant: a pawn moving onto an empty square captured the pawn that
    // just passed it, sitting beside the pawn's origin.
    if piece.kind == Kind::Pawn && from % 8 != to % 8 && board[to].is_none() {
        next[(from / 8) * 8 + to % 8] = None;
    }

    // Castling: the king moved two files; park the rook on its inside square.
    if piece.kind == Kind::King && (from % 8).abs_diff(to % 8) == 2 {
        let row = from / 8;
        if to % 8 == 6 {
            next[row * 8 + 5] = next[row * 8 + 7];
            next[row * 8 + 7] = None;
        } else {
            next[row * 8 + 3] = next[row * 8 + 0];
            next[row * 8 + 0] = None;
        }
    }

    // Promotion.
    if uci.len() > 4 {
        let kind = match uci.as_bytes()[4] {
            b'r' => Kind::Rook,
            b'n' => Kind::Knight,
            b'b' => Kind::Bishop,
            _ => Kind::Queen,
        };
        next[to] = Some(Piece { color: piece.color, kind });
    }
    next
}

/// A puzzle record, as emitted by pick.sh (flat JSON).
#[derive(Clone, Debug, Deserialize)]
pub struct PuzzleRecord {
    pub id: String,
    pub fen: String,
    pub rating: i64,
    #[serde(default)]
    pub themes: Vec<String>,
    pub solution: Vec<String>,
}

pub fn side_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}
