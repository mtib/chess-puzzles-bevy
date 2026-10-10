//! Panel state machine — mirrors the QML Panel.qml logic one-to-one:
//! phases, click-click input judged against the solution list, auto-played
//! opponent replies, solution playback, timers, user arrows, persistence.

use crate::model::{apply_uci, index_to_square, parse_fen, side_name, Color, Piece, PuzzleRecord};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

pub const ELO_MIN: i64 = 600;
pub const ELO_MAX: i64 = 2500;
pub const ELO_STEP: i64 = 50;
const STATE_FILE: &str = ".local/state/omarchy/settings/chesspuzzles-bevy.json";
const PICK_SH: &str = ".config/omarchy/plugins/mtib.chesspuzzles/pick.sh";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Loading,
    Play,
    Replying,
    Won,
    Revealed,
}

#[derive(Clone, Copy)]
pub struct Arrow {
    pub from: usize,
    pub to: usize,
    pub alpha: f32,
    pub age_ms: f32, // show 900ms, then fade 500ms
}

pub struct PanelState {
    pub puzzle: Option<PuzzleRecord>,
    pub board: [Option<Piece>; 64],
    pub user_color: Color,
    pub flip_view: bool,
    pub step: usize,
    pub tries: u32,
    pub selected: Option<usize>,
    pub wrong_flash: Option<usize>,
    pub last_from: Option<usize>,
    pub last_to: Option<usize>,
    pub user_arrows: Vec<(usize, usize)>,
    pub drag_arrow: Option<(usize, usize)>,
    pub auto_arrow: Option<Arrow>,
    pub phase: Phase,
    pub status: String,
    pub target_elo: i64,
    pub solved: u32,
    // countdowns in ms
    pub reply_timer: f32,
    pub solution_timer: f32,
    pub flash_timer: f32,
    pub fetch_retries: u32,
}

impl PanelState {
    pub fn new() -> Self {
        let (elo, solved) = load_state();
        PanelState {
            puzzle: None,
            board: [None; 64],
            user_color: Color::White,
            flip_view: false,
            step: 0,
            tries: 0,
            selected: None,
            wrong_flash: None,
            last_from: None,
            last_to: None,
            user_arrows: Vec::new(),
            drag_arrow: None,
            auto_arrow: None,
            phase: Phase::Loading,
            status: "Fetching puzzle…".into(),
            target_elo: elo,
            solved,
            reply_timer: 0.0,
            solution_timer: 0.0,
            flash_timer: 0.0,
            fetch_retries: 0,
        }
    }

    pub fn solution(&self) -> &[String] {
        self.puzzle.as_ref().map(|p| p.solution.as_slice()).unwrap_or(&[])
    }

    pub fn show_arrow(&mut self, from: usize, to: usize) {
        self.auto_arrow = Some(Arrow { from, to, alpha: 1.0, age_ms: 0.0 });
    }

    fn win(&mut self) {
        self.phase = Phase::Won;
        self.solved += 1;
        save_state(self);
        self.status = if self.tries == 0 {
            "Solved! Clean solve 🎉".into()
        } else {
            format!(
                "Solved! ({} wrong {})",
                self.tries,
                if self.tries == 1 { "try" } else { "tries" }
            )
        };
    }

    pub fn fetch_puzzle(&mut self) {
        match fetch_puzzle(self.target_elo) {
            Some(record) => {
                let (board, turn) = parse_fen(&record.fen);
                self.puzzle = Some(record);
                self.board = board;
                self.user_color = turn;
                self.step = 0;
                self.tries = 0;
                self.selected = None;
                self.wrong_flash = None;
                self.last_from = None;
                self.last_to = None;
                self.user_arrows.clear();
                self.drag_arrow = None;
                self.auto_arrow = None;
                self.phase = Phase::Play;
                self.fetch_retries = 0;
                self.status = format!("{} to play — find the best move", side_name(turn));
            }
            None => {
                self.phase = Phase::Loading;
                self.status = "Could not get a puzzle — check the connection".into();
                self.fetch_retries += 1;
            }
        }
    }

    pub fn click_square(&mut self, index: usize) {
        if self.phase != Phase::Play {
            return;
        }
        // A plain click on an empty square clears the drawn arrows.
        if self.board[index].is_none() {
            self.user_arrows.clear();
        }
        match self.selected {
            None => {
                if let Some(piece) = self.board[index] {
                    if piece.color == self.user_color {
                        self.selected = Some(index);
                    }
                }
            }
            Some(sel) if sel == index => {
                self.selected = None;
            }
            Some(sel) => self.attempt_move(sel, index),
        }
    }

    fn attempt_move(&mut self, from: usize, to: usize) {
        if self.phase != Phase::Play {
            return;
        }
        let uci = format!("{}{}", index_to_square(from), index_to_square(to));
        let expected = self.solution().get(self.step).cloned().unwrap_or_default();
        if !expected.starts_with(&uci) {
            self.tries += 1;
            self.wrong_flash = Some(to);
            self.flash_timer = 450.0;
            self.status = "Not the move — try again".into();
            self.selected = None;
            return;
        }
        self.board = apply_uci(&self.board, &expected);
        self.last_from = Some(from);
        self.last_to = Some(to);
        self.show_arrow(from, to);
        self.selected = None;
        self.step += 1;

        if self.step >= self.solution().len() {
            self.win();
            return;
        }
        self.phase = Phase::Replying;
        self.status = "Correct! Opponent replies…".into();
        self.reply_timer = 650.0;
    }

    pub fn play_reply(&mut self) {
        let Some(mv) = self.solution().get(self.step).cloned() else {
            return;
        };
        let from = crate::model::square_to_index(&mv[0..2]);
        let to = crate::model::square_to_index(&mv[2..4]);
        self.board = apply_uci(&self.board, &mv);
        self.last_from = Some(from);
        self.last_to = Some(to);
        self.show_arrow(from, to);
        self.step += 1;
        if self.step >= self.solution().len() {
            self.win();
        } else {
            self.phase = Phase::Play;
            self.status = format!("{} to play", side_name(self.user_color));
        }
    }

    pub fn play_solution_step(&mut self) {
        let Some(mv) = self.solution().get(self.step).cloned() else {
            return;
        };
        let from = crate::model::square_to_index(&mv[0..2]);
        let to = crate::model::square_to_index(&mv[2..4]);
        self.board = apply_uci(&self.board, &mv);
        self.last_from = Some(from);
        self.last_to = Some(to);
        self.show_arrow(from, to);
        self.step += 1;
        if self.step >= self.solution().len() {
            let id = self.puzzle.as_ref().map(|p| p.id.clone()).unwrap_or_default();
            self.status = format!("Puzzle {id} revealed");
        } else {
            self.solution_timer = 650.0;
        }
    }

    pub fn show_solution(&mut self) {
        if self.puzzle.is_none() || self.step >= self.solution().len() {
            return;
        }
        self.phase = Phase::Revealed;
        self.status = "Solution".into();
        self.solution_timer = 650.0;
    }

    pub fn set_elo(&mut self, elo: i64) {
        let elo = elo.clamp(ELO_MIN, ELO_MAX);
        if elo == self.target_elo {
            return;
        }
        self.target_elo = elo;
        save_state(self);
        self.fetch_puzzle();
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    elo: i64,
    solved: u32,
}

fn state_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(STATE_FILE))
        .unwrap_or_else(|_| PathBuf::from(STATE_FILE))
}

fn load_state() -> (i64, u32) {
    let Ok(raw) = std::fs::read_to_string(state_path()) else {
        return (1500, 0);
    };
    match serde_json::from_str::<PersistedState>(&raw) {
        Ok(p) if (ELO_MIN..=ELO_MAX).contains(&p.elo) => (p.elo, p.solved),
        _ => (1500, 0),
    }
}

fn save_state(state: &PanelState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = PersistedState { elo: state.target_elo, solved: state.solved };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = std::fs::write(path, json);
    }
}

fn fetch_puzzle(target_elo: i64) -> Option<PuzzleRecord> {
    let script = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(PICK_SH))
        .unwrap_or_else(|_| PathBuf::from(PICK_SH));
    let out = Command::new(&script).arg(target_elo.to_string()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice::<PuzzleRecord>(&out.stdout).ok()
}
