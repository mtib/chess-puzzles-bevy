//! Panel state machine — mirrors the QML Panel.qml logic: phases, click-click
//! input judged against the solution list, auto-played opponent replies,
//! solution playback, user arrows, persistence.
//!
//! The target ELO is hidden and adapts: every puzzle settles exactly once —
//! a clean solve raises it, a solve with mistakes lowers it, revealing the
//! solution (or skipping) lowers it more.
//!
//! Pending moves live inside the phase that owns them (`Replying`,
//! `Revealing`), so leaving a phase cancels its countdown. Puzzle fetching is
//! only *requested* here; main.rs runs the picker off the main thread and
//! hands the result back through `puzzle_loaded`.

use crate::model::{apply_uci, index_to_square, parse_fen, side_name, square_to_index, Color, Piece, PuzzleRecord};
use crate::paths::resource_dir;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

pub const ELO_MIN: i64 = 600;
pub const ELO_MAX: i64 = 2500;
const DEFAULT_ELO: i64 = 1500;
/// Target changes per settled puzzle. They balance out where about two in
/// three puzzles are solved cleanly.
const CLEAN_GAIN: i64 = 8;
const MISTAKES_LOSS: i64 = 4;
const REVEAL_LOSS: i64 = 12;
const STATE_FILE: &str = ".local/state/omarchy/settings/chesspuzzles-bevy.json";

const REPLY_DELAY_MS: f32 = 650.0;
const SOLUTION_STEP_MS: f32 = 650.0;
const WRONG_FLASH_MS: f32 = 450.0;
const ARROW_HOLD_MS: f32 = 900.0;
const ARROW_FADE_MS: f32 = 500.0;
const RETRY_DELAY_MS: f32 = 2000.0;
const MAX_FETCH_RETRIES: u32 = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// A puzzle fetch is pending or running.
    Loading,
    /// The last fetch failed; refetches automatically when `retry_in_ms` runs out.
    Failed { retry_in_ms: Option<f32> },
    Play,
    /// The user's move was right; the opponent's reply plays when `in_ms` runs out.
    Replying { in_ms: f32 },
    Won,
    /// The solution is playing back; the next move plays when `in_ms` runs out.
    Revealing { in_ms: f32 },
    Revealed,
}

/// How the status line reads: plain prose, muted, or a bold status word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Muted,
    Good,
    Bad,
}

/// How a puzzle ended, for the target ELO.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Clean,
    Mistakes,
    /// Solution shown, or skipped mid-puzzle.
    Revealed,
}

impl Outcome {
    fn delta(self) -> i64 {
        match self {
            Outcome::Clean => CLEAN_GAIN,
            Outcome::Mistakes => -MISTAKES_LOSS,
            Outcome::Revealed => -REVEAL_LOSS,
        }
    }
}

/// "+8", "−4", "±0" (the last when the target is pinned at a bound).
pub fn format_change(delta: i64) -> String {
    match delta {
        0 => "±0".into(),
        d if d > 0 => format!("+{d}"),
        d => format!("−{}", -d),
    }
}

#[derive(Clone, Copy)]
pub struct Arrow {
    pub from: usize,
    pub to: usize,
    pub alpha: f32,
    age_ms: f32,
}

pub struct PanelState {
    pub puzzle: Option<PuzzleRecord>,
    pub board: [Option<Piece>; 64],
    pub user_color: Color,
    pub flip_view: bool,
    pub step: usize,
    pub tries: u32,
    pub selected: Option<usize>,
    /// The square of a wrong attempt, and how long it stays flagged.
    pub wrong_flash: Option<(usize, f32)>,
    pub last_move: Option<(usize, usize)>,
    pub user_arrows: Vec<(usize, usize)>,
    pub drag_arrow: Option<(usize, usize)>,
    pub auto_arrow: Option<Arrow>,
    pub phase: Phase,
    pub status: String,
    pub tone: Tone,
    /// Hidden: drives puzzle selection, never shown.
    pub target_elo: i64,
    pub solved: u32,
    /// The target change this puzzle settled with; `None` while unsettled.
    pub change: Option<i64>,
    fetch_requested: bool,
    fetch_retries: u32,
}

impl PanelState {
    /// Fresh state from the persisted settings, with a puzzle fetch requested.
    pub fn new() -> Self {
        let (elo, solved) = load_state();
        Self::with_settings(elo, solved)
    }

    pub fn with_settings(target_elo: i64, solved: u32) -> Self {
        let mut state = PanelState {
            puzzle: None,
            board: [None; 64],
            user_color: Color::White,
            flip_view: false,
            step: 0,
            tries: 0,
            selected: None,
            wrong_flash: None,
            last_move: None,
            user_arrows: Vec::new(),
            drag_arrow: None,
            auto_arrow: None,
            phase: Phase::Loading,
            status: String::new(),
            tone: Tone::Muted,
            target_elo,
            solved,
            change: None,
            fetch_requested: false,
            fetch_retries: 0,
        };
        state.request_puzzle();
        state
    }

    /// The board is drawn upside down: black to move, toggled by the flip key.
    pub fn is_flipped(&self) -> bool {
        (self.user_color == Color::Black) != self.flip_view
    }

    pub fn solution(&self) -> &[String] {
        self.puzzle.as_ref().map(|p| p.solution.as_slice()).unwrap_or(&[])
    }

    fn set_status(&mut self, text: impl Into<String>, tone: Tone) {
        self.status = text.into();
        self.tone = tone;
    }

    // ------------------------------------------------------------- fetching

    /// Asks for a new puzzle at the target ELO. Cancels whatever was going
    /// on; leaving a puzzle unfinished counts as revealing it.
    pub fn request_puzzle(&mut self) {
        let skipped = self.in_puzzle().then(|| self.settle(Outcome::Revealed));
        self.fetch_requested = true;
        self.phase = Phase::Loading;
        self.selected = None;
        self.drag_arrow = None;
        match skipped {
            Some(delta) => self.set_status(format!("skipped {} · loading puzzle…", format_change(delta)), Tone::Muted),
            None => self.set_status("loading puzzle…", Tone::Muted),
        }
    }

    /// A puzzle is on the board and its outcome isn't decided yet.
    fn in_puzzle(&self) -> bool {
        matches!(self.phase, Phase::Play | Phase::Replying { .. }) && self.change.is_none()
    }

    /// Applies the outcome to the target ELO, once per puzzle. Returns the
    /// change actually applied (clamped at the bounds).
    fn settle(&mut self, outcome: Outcome) -> i64 {
        if let Some(delta) = self.change {
            return delta;
        }
        let before = self.target_elo;
        self.target_elo = (before + outcome.delta()).clamp(ELO_MIN, ELO_MAX);
        let delta = self.target_elo - before;
        self.change = Some(delta);
        save_state(self);
        delta
    }

    /// True while a request is waiting to be picked up.
    pub fn fetch_pending(&self) -> bool {
        self.fetch_requested
    }

    /// Hands the pending request (its target ELO) to the fetcher.
    pub fn take_fetch_request(&mut self) -> Option<i64> {
        std::mem::take(&mut self.fetch_requested).then_some(self.target_elo)
    }

    pub fn puzzle_loaded(&mut self, record: Option<PuzzleRecord>) {
        let Some(record) = record else {
            self.fetch_retries += 1;
            let retry_in_ms = (self.fetch_retries <= MAX_FETCH_RETRIES).then_some(RETRY_DELAY_MS);
            self.phase = Phase::Failed { retry_in_ms };
            let what = if retry_in_ms.is_some() { "NO PUZZLE · RETRYING" } else { "NO PUZZLE · CHECK THE CONNECTION" };
            self.set_status(what, Tone::Bad);
            return;
        };
        let (board, turn) = parse_fen(&record.fen);
        self.puzzle = Some(record);
        self.board = board;
        self.user_color = turn;
        self.step = 0;
        self.tries = 0;
        self.selected = None;
        self.wrong_flash = None;
        self.last_move = None;
        self.user_arrows.clear();
        self.drag_arrow = None;
        self.auto_arrow = None;
        self.phase = Phase::Play;
        self.change = None;
        self.fetch_retries = 0;
        self.set_status(format!("{} to play · find the best move", side_name(turn)), Tone::Normal);
    }

    // ------------------------------------------------------------- playing

    pub fn click_square(&mut self, index: usize) {
        // A plain click on an empty square clears the drawn arrows.
        if self.board[index].is_none() {
            self.user_arrows.clear();
        }
        if self.phase != Phase::Play {
            return;
        }
        match self.selected {
            None => {
                if self.board[index].is_some_and(|p| p.color == self.user_color) {
                    self.selected = Some(index);
                }
            }
            Some(sel) if sel == index => self.selected = None,
            Some(sel) => self.attempt_move(sel, index),
        }
    }

    fn attempt_move(&mut self, from: usize, to: usize) {
        self.selected = None;
        let uci = format!("{}{}", index_to_square(from), index_to_square(to));
        let expected = self.solution().get(self.step).cloned().unwrap_or_default();
        // Promotions are accepted on the first four characters.
        if !expected.starts_with(&uci) {
            self.tries += 1;
            self.wrong_flash = Some((to, WRONG_FLASH_MS));
            self.set_status("NOT THE MOVE · TRY AGAIN", Tone::Bad);
            return;
        }
        self.play(&expected);
        if self.step >= self.solution().len() {
            self.win();
        } else {
            self.phase = Phase::Replying { in_ms: REPLY_DELAY_MS };
            self.set_status("CORRECT · OPPONENT REPLIES…", Tone::Normal);
        }
    }

    /// Plays one solution move onto the board and marks it.
    fn play(&mut self, uci: &str) {
        let from = square_to_index(&uci[0..2]);
        let to = square_to_index(&uci[2..4]);
        self.board = apply_uci(&self.board, uci);
        self.last_move = Some((from, to));
        self.auto_arrow = Some(Arrow { from, to, alpha: 1.0, age_ms: 0.0 });
        self.step += 1;
    }

    fn next_solution_move(&self) -> Option<String> {
        self.solution().get(self.step).cloned()
    }

    fn play_reply(&mut self) {
        let Some(mv) = self.next_solution_move() else { return };
        self.play(&mv);
        if self.step >= self.solution().len() {
            self.win();
        } else {
            self.phase = Phase::Play;
            self.set_status(format!("{} to play", side_name(self.user_color)), Tone::Normal);
        }
    }

    fn play_solution_step(&mut self) {
        let Some(mv) = self.next_solution_move() else { return };
        self.play(&mv);
        if self.step >= self.solution().len() {
            self.phase = Phase::Revealed;
            let change = format_change(self.change.unwrap_or_default());
            self.set_status(format!("REVEALED · {change}"), Tone::Muted);
        } else {
            self.phase = Phase::Revealing { in_ms: SOLUTION_STEP_MS };
        }
    }

    fn win(&mut self) {
        self.phase = Phase::Won;
        self.solved += 1;
        let outcome = if self.tries == 0 { Outcome::Clean } else { Outcome::Mistakes };
        let change = format_change(self.settle(outcome));
        let status = match self.tries {
            0 => format!("SOLVED · CLEAN · {change}"),
            1 => format!("SOLVED · 1 WRONG TRY · {change}"),
            n => format!("SOLVED · {n} WRONG TRIES · {change}"),
        };
        self.set_status(status, Tone::Good);
    }

    /// Whether "show solution" does anything right now.
    pub fn can_show_solution(&self) -> bool {
        matches!(self.phase, Phase::Play | Phase::Replying { .. }) && self.step < self.solution().len()
    }

    /// Plays the rest of the line back. Replaces a pending reply: the reply
    /// is simply the next move of the playback.
    pub fn show_solution(&mut self) {
        if !self.can_show_solution() {
            return;
        }
        self.selected = None;
        let change = format_change(self.settle(Outcome::Revealed));
        self.phase = Phase::Revealing { in_ms: SOLUTION_STEP_MS };
        self.set_status(format!("SOLUTION · {change}"), Tone::Normal);
    }

    pub fn toggle_flip(&mut self) {
        self.flip_view = !self.flip_view;
    }

    // ------------------------------------------------------------- arrows

    pub fn begin_arrow(&mut self, index: usize) {
        self.drag_arrow = Some((index, index));
    }

    pub fn drag_arrow_to(&mut self, index: usize) {
        if let Some((from, _)) = self.drag_arrow {
            self.drag_arrow = Some((from, index));
        }
    }

    pub fn end_arrow(&mut self) {
        if let Some((from, to)) = self.drag_arrow.take()
            && from != to
        {
            self.user_arrows.push((from, to));
        }
    }

    // ------------------------------------------------------------- time

    /// True while something changes on its own (a countdown or a fade), so
    /// the app must keep rendering frames.
    pub fn is_animating(&self) -> bool {
        matches!(
            self.phase,
            Phase::Replying { .. } | Phase::Revealing { .. } | Phase::Failed { retry_in_ms: Some(_) }
        ) || self.wrong_flash.is_some()
            || self.auto_arrow.is_some()
    }

    /// Advances every countdown by `dt_ms`.
    pub fn tick(&mut self, dt_ms: f32) {
        match &mut self.phase {
            Phase::Replying { in_ms } => {
                *in_ms -= dt_ms;
                if *in_ms <= 0.0 {
                    self.play_reply();
                }
            }
            Phase::Revealing { in_ms } => {
                *in_ms -= dt_ms;
                if *in_ms <= 0.0 {
                    self.play_solution_step();
                }
            }
            Phase::Failed { retry_in_ms: Some(in_ms) } => {
                *in_ms -= dt_ms;
                if *in_ms <= 0.0 {
                    self.request_puzzle();
                }
            }
            _ => {}
        }
        if let Some((_, left)) = &mut self.wrong_flash {
            *left -= dt_ms;
            if *left <= 0.0 {
                self.wrong_flash = None;
            }
        }
        if let Some(arrow) = &mut self.auto_arrow {
            arrow.age_ms += dt_ms;
            arrow.alpha = (1.0 - (arrow.age_ms - ARROW_HOLD_MS) / ARROW_FADE_MS).clamp(0.0, 1.0);
            if arrow.alpha <= 0.0 {
                self.auto_arrow = None;
            }
        }
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
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<PersistedState>(&raw).ok())
        .filter(|p| (ELO_MIN..=ELO_MAX).contains(&p.elo))
        .map_or((DEFAULT_ELO, 0), |p| (p.elo, p.solved))
}

fn save_state(state: &PanelState) {
    if cfg!(test) {
        return;
    }
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = PersistedState { elo: state.target_elo, solved: state.solved };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = std::fs::write(path, json);
    }
}

/// Puzzle picker: $CHESS_PUZZLES_PICK, else the bundled scripts/pick.sh, else
/// the Omarchy plugin's copy.
fn pick_script() -> PathBuf {
    if let Ok(path) = std::env::var("CHESS_PUZZLES_PICK") {
        return PathBuf::from(path);
    }
    let bundled = resource_dir().join("scripts/pick.sh");
    if bundled.exists() {
        return bundled;
    }
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/omarchy/plugins/mtib.chesspuzzles/pick.sh"))
        .unwrap_or(bundled)
}

/// Runs the picker. Blocking (it may hit the network): call off the main thread.
pub fn fetch_puzzle(target_elo: i64) -> Option<PuzzleRecord> {
    let out = Command::new(pick_script()).arg(target_elo.to_string()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice::<PuzzleRecord>(&out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // White to move; solution: Qd8+ (d1d8), Kh7 (g8h7), Qh4# (d8h4).
    fn loaded() -> PanelState {
        let mut s = PanelState::with_settings(1500, 0);
        s.take_fetch_request();
        s.puzzle_loaded(Some(PuzzleRecord {
            id: "test1".into(),
            fen: "6k1/5ppp/8/8/8/8/5PPP/3Q2K1 w - - 0 1".into(),
            rating: 1500,
            themes: vec![],
            solution: vec!["d1d8".into(), "g8h7".into(), "d8h4".into()],
        }));
        s
    }

    fn click(s: &mut PanelState, from: &str, to: &str) {
        s.click_square(square_to_index(from));
        s.click_square(square_to_index(to));
    }

    #[test]
    fn solving_plays_the_reply_then_wins() {
        let mut s = loaded();
        click(&mut s, "d1", "d8");
        assert!(matches!(s.phase, Phase::Replying { .. }));
        s.tick(REPLY_DELAY_MS);
        assert_eq!(s.phase, Phase::Play);
        click(&mut s, "d8", "h4");
        assert_eq!(s.phase, Phase::Won);
        assert_eq!(s.solved, 1);
    }

    #[test]
    fn show_solution_during_reply_plays_each_move_once() {
        let mut s = loaded();
        click(&mut s, "d1", "d8");
        s.show_solution();
        for _ in 0..10 {
            s.tick(SOLUTION_STEP_MS);
        }
        assert_eq!(s.phase, Phase::Revealed);
        assert_eq!(s.step, 3);
        assert_eq!(s.solved, 0, "revealing must not count as a solve");
    }

    #[test]
    fn new_puzzle_cancels_playback() {
        let mut s = loaded();
        s.show_solution();
        s.request_puzzle();
        s.tick(SOLUTION_STEP_MS * 4.0);
        assert_eq!(s.phase, Phase::Loading);
        assert_eq!(s.step, 0);
    }

    #[test]
    fn flip_combines_side_and_toggle() {
        let mut s = loaded();
        assert!(!s.is_flipped());
        s.toggle_flip();
        assert!(s.is_flipped());
        s.user_color = Color::Black;
        assert!(!s.is_flipped(), "black to move, flipped back by the toggle");
    }

    fn solve(s: &mut PanelState) {
        click(s, "d1", "d8");
        s.tick(REPLY_DELAY_MS);
        click(s, "d8", "h4");
        assert_eq!(s.phase, Phase::Won);
    }

    #[test]
    fn clean_solve_raises_target() {
        let mut s = loaded();
        solve(&mut s);
        assert_eq!(s.target_elo, 1500 + CLEAN_GAIN);
    }

    #[test]
    fn solve_with_mistakes_lowers_target() {
        let mut s = loaded();
        click(&mut s, "d1", "d7");
        click(&mut s, "d1", "d6");
        solve(&mut s);
        assert_eq!(s.tries, 2);
        assert_eq!(s.target_elo, 1500 - MISTAKES_LOSS, "lowered once, not per mistake");
    }

    #[test]
    fn reveal_lowers_more_and_only_once() {
        let mut s = loaded();
        click(&mut s, "d1", "d7");
        s.show_solution();
        for _ in 0..10 {
            s.tick(SOLUTION_STEP_MS);
        }
        assert_eq!(s.phase, Phase::Revealed);
        assert_eq!(s.target_elo, 1500 - REVEAL_LOSS, "mistakes before a reveal add nothing");
        s.request_puzzle();
        assert_eq!(s.target_elo, 1500 - REVEAL_LOSS, "moving on from a settled puzzle is free");
    }

    #[test]
    fn skipping_mid_puzzle_counts_as_reveal() {
        let mut s = loaded();
        click(&mut s, "d1", "d8"); // right move, reply pending
        s.request_puzzle();
        assert_eq!(s.target_elo, 1500 - REVEAL_LOSS);
        assert_eq!(s.take_fetch_request(), Some(1500 - REVEAL_LOSS), "the next puzzle uses the new target");
    }

    #[test]
    fn loading_and_retrying_are_free() {
        let mut s = PanelState::with_settings(1500, 0);
        s.request_puzzle();
        s.take_fetch_request();
        s.puzzle_loaded(None);
        s.tick(RETRY_DELAY_MS);
        assert_eq!(s.target_elo, 1500);
    }

    #[test]
    fn target_stays_in_bounds() {
        let mut s = PanelState::with_settings(ELO_MIN + 3, 0);
        s.take_fetch_request();
        s.puzzle_loaded(loaded().puzzle);
        s.show_solution();
        assert_eq!(s.target_elo, ELO_MIN);
        assert_eq!(s.change, Some(-3));
        assert_eq!(format_change(0), "±0");
    }

    #[test]
    fn failed_fetch_retries_then_gives_up() {
        let mut s = PanelState::with_settings(1500, 0);
        for _ in 0..=MAX_FETCH_RETRIES {
            assert!(s.take_fetch_request().is_some());
            s.puzzle_loaded(None);
            s.tick(RETRY_DELAY_MS);
        }
        assert_eq!(s.phase, Phase::Failed { retry_in_ms: None });
        assert!(s.take_fetch_request().is_none());
    }
}
