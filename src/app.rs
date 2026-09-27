//! Application state: game dispatch, menu, cursor, undo, hints, stats.
//!
//! [`App`] owns one of each game plus the TUI session state: which screen is
//! visible, the difficulty, the cursor/selection for keyboard play, per-game
//! undo stacks, and the status line.

use std::collections::VecDeque;
use std::time::Instant;

use crate::freecell::FreeCellGame;
use crate::klondike::{Difficulty, DrawMode, KlondikeGame};
use crate::spider::{SpiderGame, SpiderSuits};

/// Which solitaire variant is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Klondike,
    FreeCell,
    SpiderMini,
}

impl GameMode {
    pub fn all() -> [GameMode; 3] {
        [GameMode::Klondike, GameMode::FreeCell, GameMode::SpiderMini]
    }

    pub fn title(self) -> &'static str {
        match self {
            GameMode::Klondike => "Klondike",
            GameMode::FreeCell => "FreeCell",
            GameMode::SpiderMini => "Spider-mini",
        }
    }
}

/// Which screen is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Game,
    Menu,
}

/// Cursor pile area. Not every area exists in every game (checked by
/// [`App::area_size`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorArea {
    Tableau,
    Foundation,
    FreeCell,
    Stock,
    Waste,
}

/// Cursor position: a pile area plus an index inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub area: CursorArea,
    pub index: usize,
}

/// A grabbed pile ready to place elsewhere. `count` is the tableau fan size;
/// single-card sources always use 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    pub area: CursorArea,
    pub index: usize,
    pub count: usize,
}

/// Cursor step direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

const MAX_UNDO: usize = 300;

/// A destructive action (discards the current game or exits) awaiting a
/// second keypress to confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingConfirm {
    Quit,
    NewGame,
    Restart,
}

/// Top-level app state.
#[derive(Debug)]
pub struct App {
    pub mode: GameMode,
    pub screen: Screen,
    pub difficulty: Difficulty,
    pub draw_mode: DrawMode,
    pub klondike: KlondikeGame,
    pub freecell: FreeCellGame,
    pub spider: SpiderGame,
    pub cursor: Cursor,
    pub selected: Option<Selected>,
    pub menu_index: usize,
    pub menu_diff_index: usize,
    pub show_help: bool,
    pub status_message: String,
    pub should_quit: bool,
    pub started_at: Instant,
    pub confirm: Option<PendingConfirm>,
    undo_klondike: VecDeque<KlondikeGame>,
    undo_freecell: VecDeque<FreeCellGame>,
    undo_spider: VecDeque<SpiderGame>,
}

impl Default for App {
    fn default() -> Self {
        Self::new(DrawMode::Draw3, Difficulty::Normal)
    }
}

impl App {
    pub fn new(draw_mode: DrawMode, difficulty: Difficulty) -> Self {
        let spider_suits = SpiderSuits::from_difficulty(difficulty);
        Self {
            mode: GameMode::Klondike,
            screen: Screen::Menu,
            difficulty,
            draw_mode,
            klondike: KlondikeGame::new(draw_mode, difficulty),
            freecell: FreeCellGame::new_with_difficulty(difficulty),
            spider: SpiderGame::new(spider_suits),
            cursor: Cursor {
                area: CursorArea::Tableau,
                index: 0,
            },
            selected: None,
            menu_index: 0,
            menu_diff_index: difficulty_menu_index(difficulty),
            show_help: false,
            status_message: String::from("Menu: up/down picks a game, left/right sets difficulty."),
            should_quit: false,
            started_at: Instant::now(),
            confirm: None,
            undo_klondike: VecDeque::new(),
            undo_freecell: VecDeque::new(),
            undo_spider: VecDeque::new(),
        }
    }

    // ---- mode / screen ----

    /// Switch mode directly, bypassing the menu. Production code always
    /// goes through `menu_confirm`; this is a test-setup convenience.
    #[cfg(test)]
    pub fn switch_mode(&mut self, mode: GameMode) {
        self.mode = mode;
        self.selected = None;
        self.ensure_cursor_valid();
        self.status_message = format!("Switched to {}.", mode.title());
    }

    pub fn open_menu(&mut self) {
        self.screen = Screen::Menu;
        self.menu_index = mode_menu_index(self.mode);
        self.menu_diff_index = difficulty_menu_index(self.difficulty);
        self.status_message = "Menu: up/down picks a game, left/right sets difficulty.".to_string();
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    // ---- destructive-action confirmation ----

    /// Ask for confirmation before quitting (q/Esc on the table).
    pub fn request_quit(&mut self) {
        self.confirm = Some(PendingConfirm::Quit);
        self.status_message = "Quit Solitaire? (y to confirm, any other key to cancel)".to_string();
    }

    /// Ask for confirmation before discarding the game for a fresh deal.
    pub fn request_new_game(&mut self) {
        self.confirm = Some(PendingConfirm::NewGame);
        self.status_message =
            "Discard this game and deal a new one? (y to confirm, any other key to cancel)"
                .to_string();
    }

    /// Ask for confirmation before restarting with a fresh shuffle.
    pub fn request_restart(&mut self) {
        self.confirm = Some(PendingConfirm::Restart);
        self.status_message =
            "Restart with a fresh shuffle? (y to confirm, any other key to cancel)".to_string();
    }

    /// y/Enter on a pending confirmation: carry out the action.
    pub fn confirm_pending(&mut self) {
        match self.confirm.take() {
            Some(PendingConfirm::Quit) => self.quit(),
            Some(PendingConfirm::NewGame) => self.new_game(),
            Some(PendingConfirm::Restart) => self.restart(),
            None => {}
        }
    }

    /// Any other key on a pending confirmation: back out, nothing changed.
    pub fn cancel_pending(&mut self) {
        if self.confirm.take().is_some() {
            self.status_message = "Cancelled.".to_string();
        }
    }

    // ---- menu ----

    pub fn menu_move(&mut self, dir: Dir) {
        match dir {
            Dir::Up => {
                self.menu_index = self.menu_index.saturating_sub(1);
            }
            Dir::Down => {
                self.menu_index = (self.menu_index + 1).min(GameMode::all().len() - 1);
            }
            Dir::Left => {
                self.menu_diff_index = self.menu_diff_index.saturating_sub(1);
            }
            Dir::Right => {
                self.menu_diff_index = (self.menu_diff_index + 1).min(2);
            }
        }
    }

    /// Confirm the menu: apply game + difficulty, deal fresh for modes whose
    /// layout depends on it, return to the table.
    pub fn menu_confirm(&mut self) {
        let mode = GameMode::all()[self.menu_index];
        let difficulty = difficulty_from_menu_index(self.menu_diff_index);
        let diff_changed = difficulty != self.difficulty;
        self.difficulty = difficulty;
        self.mode = mode;
        // Klondike difficulty (redeal rules) applies live; the current deal
        // is kept as-is rather than reshuffled.
        self.klondike.difficulty = difficulty;
        match mode {
            GameMode::Klondike => {}
            GameMode::FreeCell => {
                if diff_changed {
                    self.freecell = FreeCellGame::new_with_difficulty(difficulty);
                    self.undo_freecell.clear();
                }
            }
            GameMode::SpiderMini => {
                if diff_changed {
                    self.spider = SpiderGame::new(SpiderSuits::from_difficulty(difficulty));
                    self.undo_spider.clear();
                }
            }
        }
        self.screen = Screen::Game;
        self.selected = None;
        self.ensure_cursor_valid();
        self.status_message = format!(
            "Playing {} on {}.",
            mode.title(),
            difficulty_label(difficulty)
        );
    }

    // ---- new game / restart ----

    /// Fresh shuffle of the current game for the current difficulty.
    pub fn new_game(&mut self) {
        match self.mode {
            GameMode::Klondike => {
                self.klondike.draw_mode = self.draw_mode;
                self.klondike.difficulty = self.difficulty;
                if self.difficulty == Difficulty::Easy {
                    self.klondike.reset_winnable();
                } else {
                    self.klondike.reset();
                }
                self.undo_klondike.clear();
            }
            GameMode::FreeCell => {
                self.freecell = FreeCellGame::new_with_difficulty(self.difficulty);
                self.undo_freecell.clear();
            }
            GameMode::SpiderMini => {
                self.spider = SpiderGame::new(SpiderSuits::from_difficulty(self.difficulty));
                self.undo_spider.clear();
            }
        }
        self.selected = None;
        self.ensure_cursor_valid();
        self.started_at = Instant::now();
        self.status_message = format!("New {} game dealt.", self.mode.title());
    }

    /// Restart = fresh deal with the same settings (no seed is stored, so a
    /// byte-identical redeal is not possible; this deals a new shuffle).
    pub fn restart(&mut self) {
        self.new_game();
        self.status_message = format!("Restarted {} (fresh shuffle).", self.mode.title());
    }

    // ---- undo ----

    fn push_undo(&mut self) {
        match self.mode {
            GameMode::Klondike => {
                if self.undo_klondike.len() >= MAX_UNDO {
                    self.undo_klondike.pop_front();
                }
                self.undo_klondike.push_back(self.klondike.clone());
            }
            GameMode::FreeCell => {
                if self.undo_freecell.len() >= MAX_UNDO {
                    self.undo_freecell.pop_front();
                }
                self.undo_freecell.push_back(self.freecell.clone());
            }
            GameMode::SpiderMini => {
                if self.undo_spider.len() >= MAX_UNDO {
                    self.undo_spider.pop_front();
                }
                self.undo_spider.push_back(self.spider.clone());
            }
        }
    }

    pub fn undo_depth(&self) -> usize {
        match self.mode {
            GameMode::Klondike => self.undo_klondike.len(),
            GameMode::FreeCell => self.undo_freecell.len(),
            GameMode::SpiderMini => self.undo_spider.len(),
        }
    }

    pub fn undo(&mut self) {
        let restored = match self.mode {
            GameMode::Klondike => self.undo_klondike.pop_back().map(|g| {
                self.klondike = g;
            }),
            GameMode::FreeCell => self.undo_freecell.pop_back().map(|g| {
                self.freecell = g;
            }),
            GameMode::SpiderMini => self.undo_spider.pop_back().map(|g| {
                self.spider = g;
            }),
        };
        self.selected = None;
        self.status_message = if restored.is_some() {
            "Undone.".to_string()
        } else {
            "Nothing to undo.".to_string()
        };
    }

    // ---- stats ----

    pub fn score(&self) -> i32 {
        match self.mode {
            GameMode::Klondike => self.klondike.score(),
            GameMode::FreeCell => self.freecell.score(),
            GameMode::SpiderMini => self.spider.score(),
        }
    }

    pub fn moves(&self) -> u32 {
        match self.mode {
            GameMode::Klondike => self.klondike.move_count(),
            GameMode::FreeCell => self.freecell.move_count(),
            GameMode::SpiderMini => self.spider.move_count(),
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn is_won(&self) -> bool {
        match self.mode {
            GameMode::Klondike => self.klondike.is_won(),
            GameMode::FreeCell => self.freecell.is_won(),
            GameMode::SpiderMini => self.spider.is_won(),
        }
    }

    // ---- draw / auto / hint ----

    /// Draw / deal for the active game (d key).
    pub fn draw(&mut self) {
        match self.mode {
            GameMode::Klondike => {
                let was_game_over = self.klondike.is_game_over();
                self.push_undo_silent();
                if self.klondike.draw_from_stock() {
                    self.status_message = format!(
                        "Drew. Score: {} Moves: {}.",
                        self.klondike.score(),
                        self.klondike.move_count()
                    );
                    self.check_win();
                } else if !was_game_over && self.klondike.is_game_over() {
                    // Hard-mode redeal exhaustion mutates state (sets
                    // game_over) on this same failing call, so keep the
                    // pre-draw snapshot instead of discarding it — otherwise
                    // `u` would skip past it and undo one move too many.
                    self.status_message =
                        "Game over: stock passes exhausted (u to undo).".to_string();
                } else {
                    self.undo_klondike.pop_back();
                    if self.klondike.is_won() {
                        self.status_message = "Already won!".to_string();
                    } else {
                        self.status_message = "Nothing to draw.".to_string();
                    }
                }
            }
            GameMode::SpiderMini => {
                if self.spider.stock.is_empty() {
                    self.status_message = "Stock is empty.".to_string();
                } else if self.spider.tableau.iter().any(|p| p.is_empty()) {
                    self.status_message =
                        "Fill every empty column before dealing from the stock.".to_string();
                } else {
                    self.push_undo_silent();
                    self.spider.deal_from_stock();
                    self.status_message = format!(
                        "Dealt. {} stock deals left.",
                        self.spider.stock_deals_remaining()
                    );
                    self.check_win();
                }
            }
            GameMode::FreeCell => {
                self.status_message = "No stock in FreeCell: move cards with space.".to_string();
            }
        }
    }

    /// Auto-move safe cards to the foundations (a key).
    pub fn auto_foundation(&mut self) {
        match self.mode {
            GameMode::Klondike => {
                self.push_undo_silent();
                let moved = self.klondike.auto_finish();
                if moved == 0 {
                    self.undo_klondike.pop_back();
                    self.status_message = "No cards can go to foundations right now.".to_string();
                } else {
                    self.status_message = format!("Auto-moved {moved} card(s) to foundations.");
                    self.check_win();
                }
            }
            GameMode::FreeCell => {
                self.push_undo_silent();
                let moved = self.freecell.auto_move_safe_to_foundations();
                if moved == 0 {
                    // Fall back to moving everything currently placeable.
                    let moved_all = self.freecell.auto_move_all_to_foundations();
                    if moved_all == 0 {
                        self.undo_freecell.pop_back();
                        self.status_message =
                            "No cards can go to foundations right now.".to_string();
                    } else {
                        self.status_message =
                            format!("Auto-moved {moved_all} card(s) to foundations.");
                        self.check_win();
                    }
                } else {
                    self.status_message = format!("Auto-moved {moved} safe card(s).");
                    self.check_win();
                }
            }
            GameMode::SpiderMini => {
                self.push_undo_silent();
                let moved = self.spider.auto_remove_completed();
                if moved == 0 {
                    self.undo_spider.pop_back();
                    self.status_message = "No complete K->A run to bank.".to_string();
                } else {
                    self.status_message = format!("Banked {moved} sequence(s)!");
                    self.check_win();
                }
            }
        }
    }

    /// Show a hint for the active game (h/H key).
    pub fn hint(&mut self) {
        let text = match self.mode {
            GameMode::Klondike => self.klondike.hint_text(),
            GameMode::FreeCell => self.freecell.hint_text(),
            GameMode::SpiderMini => self.spider.hint_text(),
        };
        self.status_message = text.unwrap_or_else(|| "No hint: no legal moves found.".to_string());
    }

    fn check_win(&mut self) {
        if self.is_won() {
            self.status_message = format!(
                "You win! Score {} in {} moves. (n new game, m menu)",
                self.score(),
                self.moves()
            );
        }
    }

    /// Push an undo snapshot; callers pop it back when the action fails.
    fn push_undo_silent(&mut self) {
        self.push_undo();
    }

    // ---- cursor navigation ----

    /// Pile areas in Tab order for the active game.
    fn tab_areas(&self) -> &'static [CursorArea] {
        const KLONDIKE: [CursorArea; 4] = [
            CursorArea::Stock,
            CursorArea::Waste,
            CursorArea::Foundation,
            CursorArea::Tableau,
        ];
        const FREECELL: [CursorArea; 3] = [
            CursorArea::FreeCell,
            CursorArea::Foundation,
            CursorArea::Tableau,
        ];
        const SPIDER: [CursorArea; 2] = [CursorArea::Stock, CursorArea::Tableau];
        match self.mode {
            GameMode::Klondike => &KLONDIKE,
            GameMode::FreeCell => &FREECELL,
            GameMode::SpiderMini => &SPIDER,
        }
    }

    /// Number of piles in an area for the active game.
    pub fn area_size(&self, area: CursorArea) -> usize {
        match (self.mode, area) {
            (GameMode::Klondike, CursorArea::Tableau) => 7,
            (GameMode::Klondike, CursorArea::Foundation) => 4,
            (GameMode::Klondike, CursorArea::Stock) => 1,
            (GameMode::Klondike, CursorArea::Waste) => 1,
            (GameMode::FreeCell, CursorArea::Tableau) => 8,
            (GameMode::FreeCell, CursorArea::Foundation) => 4,
            (GameMode::FreeCell, CursorArea::FreeCell) => self.freecell.num_cells,
            (GameMode::SpiderMini, CursorArea::Tableau) => 10,
            (GameMode::SpiderMini, CursorArea::Stock) => 1,
            _ => 0,
        }
    }

    pub fn ensure_cursor_valid(&mut self) {
        if self.area_size(self.cursor.area) == 0 {
            self.cursor = Cursor {
                area: CursorArea::Tableau,
                index: 0,
            };
        }
        let size = self.area_size(self.cursor.area).max(1);
        if self.cursor.index >= size {
            self.cursor.index = size - 1;
        }
    }

    pub fn move_cursor(&mut self, dir: Dir) {
        self.ensure_cursor_valid();
        let size = self.area_size(self.cursor.area).max(1);
        match dir {
            Dir::Left => {
                self.cursor.index = self.cursor.index.saturating_sub(1);
            }
            Dir::Right => {
                self.cursor.index = (self.cursor.index + 1).min(size - 1);
            }
            Dir::Up => self.tab_area_step(true),
            Dir::Down => self.tab_area_step(false),
        }
    }

    fn tab_area_step(&mut self, back: bool) {
        let areas = self.tab_areas();
        let pos = areas
            .iter()
            .position(|&a| a == self.cursor.area)
            .unwrap_or(0);
        let next = if back {
            (pos + areas.len() - 1) % areas.len()
        } else {
            (pos + 1) % areas.len()
        };
        self.cursor.area = areas[next];
        self.cursor.index = 0;
    }

    pub fn tab_next(&mut self) {
        self.tab_area_step(false);
    }

    pub fn tab_prev(&mut self) {
        self.tab_area_step(true);
    }

    pub fn jump_to_tableau(&mut self, index: usize) {
        if index < self.area_size(CursorArea::Tableau) {
            self.cursor.area = CursorArea::Tableau;
            self.cursor.index = index;
        }
    }

    pub fn focus_stock(&mut self) {
        if self.area_size(CursorArea::Stock) > 0 {
            self.cursor.area = CursorArea::Stock;
            self.cursor.index = 0;
        }
    }

    pub fn focus_waste(&mut self) {
        if self.area_size(CursorArea::Waste) > 0 {
            self.cursor.area = CursorArea::Waste;
            self.cursor.index = 0;
        }
    }

    pub fn focus_foundation(&mut self) {
        if self.area_size(CursorArea::Foundation) > 0 {
            self.cursor.area = CursorArea::Foundation;
            self.cursor.index = 0;
        }
    }

    pub fn focus_cells(&mut self) {
        if self.area_size(CursorArea::FreeCell) > 0 {
            self.cursor.area = CursorArea::FreeCell;
            self.cursor.index = 0;
        }
    }

    pub fn cancel_selection(&mut self) {
        self.selected = None;
        self.status_message = "Selection cancelled.".to_string();
    }

    // ---- select / place ----

    /// Space/Enter: grab from the cursor pile, or place a grabbed pile onto
    /// the cursor pile.
    pub fn select_or_place(&mut self) {
        if self.is_won() {
            self.status_message = "Already won! Press n for a new game.".to_string();
            return;
        }
        match self.selected {
            None => self.grab(),
            Some(sel) => {
                if sel.area == self.cursor.area && sel.index == self.cursor.index {
                    self.cancel_selection();
                } else {
                    self.place(sel);
                }
            }
        }
    }

    /// Grab cards from the cursor pile into the selection.
    fn grab(&mut self) {
        let cur = self.cursor;
        let sel = match self.mode {
            GameMode::Klondike => match cur.area {
                CursorArea::Tableau => {
                    let n = self.klondike.grab_count_tableau(cur.index);
                    if n == 0 {
                        None
                    } else {
                        Some(Selected {
                            area: cur.area,
                            index: cur.index,
                            count: n,
                        })
                    }
                }
                CursorArea::Waste => self.klondike.waste_top().map(|_| Selected {
                    area: cur.area,
                    index: 0,
                    count: 1,
                }),
                CursorArea::Foundation => {
                    self.klondike.foundation_top(cur.index).map(|_| Selected {
                        area: cur.area,
                        index: cur.index,
                        count: 1,
                    })
                }
                CursorArea::Stock => {
                    self.status_message = "Press d to draw from the stock.".to_string();
                    return;
                }
                _ => {
                    self.status_message = "Nothing to grab there.".to_string();
                    return;
                }
            },
            GameMode::FreeCell => match cur.area {
                CursorArea::Tableau => {
                    let n = self.freecell.grab_count_tableau(cur.index);
                    if n == 0 {
                        None
                    } else {
                        Some(Selected {
                            area: cur.area,
                            index: cur.index,
                            count: n,
                        })
                    }
                }
                CursorArea::FreeCell => self.freecell.cell_card(cur.index).map(|_| Selected {
                    area: cur.area,
                    index: cur.index,
                    count: 1,
                }),
                CursorArea::Foundation => {
                    self.freecell.foundation_top(cur.index).map(|_| Selected {
                        area: cur.area,
                        index: cur.index,
                        count: 1,
                    })
                }
                _ => {
                    self.status_message = "Nothing to grab there.".to_string();
                    return;
                }
            },
            GameMode::SpiderMini => match cur.area {
                CursorArea::Tableau => {
                    let n = self.spider.movable_run_len(cur.index);
                    if n == 0 {
                        None
                    } else {
                        Some(Selected {
                            area: cur.area,
                            index: cur.index,
                            count: n,
                        })
                    }
                }
                CursorArea::Stock => {
                    self.status_message = "Press d to deal from the stock.".to_string();
                    return;
                }
                _ => {
                    self.status_message = "Nothing to grab there.".to_string();
                    return;
                }
            },
        };
        match sel {
            Some(s) => {
                self.selected = Some(s);
                self.status_message = format!(
                    "Grabbed {} card(s). Move to a target pile, space to place, Esc to cancel.",
                    s.count
                );
            }
            None => {
                self.status_message = "Nothing to grab there.".to_string();
            }
        }
    }

    /// Place a grabbed pile onto the cursor pile.
    fn place(&mut self, sel: Selected) {
        let cur = self.cursor;
        self.push_undo_silent();
        let ok = match self.mode {
            GameMode::Klondike => self.place_klondike(sel, cur),
            GameMode::FreeCell => self.place_freecell(sel, cur),
            GameMode::SpiderMini => self.place_spider(sel, cur),
        };
        if ok {
            self.selected = None;
            self.check_win();
            if !self.is_won() {
                let (score, moves) = (self.score(), self.moves());
                self.status_message = format!("Moved. Score: {score} Moves: {moves}.");
            }
        } else {
            self.pop_undo();
            // keep the grab so the player can try another target
            self.status_message = self.illegal_move_reason(sel, cur);
        }
    }

    /// Explain *why* a failed placement was refused, so the player doesn't
    /// have to guess between color/rank/suit/capacity rules by trial and error.
    fn illegal_move_reason(&self, sel: Selected, cur: Cursor) -> String {
        const GENERIC: &str = "Illegal move: that card cannot go there.";
        match self.mode {
            GameMode::Klondike => {
                let moving = match sel.area {
                    CursorArea::Waste => self.klondike.waste_top(),
                    CursorArea::Foundation => self.klondike.foundation_top(sel.index),
                    CursorArea::Tableau => {
                        let pile = &self.klondike.tableau[sel.index];
                        let len = pile.len();
                        (len >= sel.count).then(|| pile[len - sel.count])
                    }
                    _ => None,
                };
                let Some(card) = moving else {
                    return GENERIC.to_string();
                };
                match cur.area {
                    CursorArea::Foundation => match self.klondike.foundation_top(cur.index) {
                        None if !card.rank.is_ace() => {
                            "Illegal move: foundations start with an Ace.".to_string()
                        }
                        Some(top) if top.suit != card.suit => {
                            "Illegal move: foundations only take the same suit, one rank up."
                                .to_string()
                        }
                        Some(top) if card.rank.value() != top.rank.value() + 1 => {
                            "Illegal move: foundations need the next rank up.".to_string()
                        }
                        _ => GENERIC.to_string(),
                    },
                    CursorArea::Tableau => match self.klondike.tableau_top(cur.index) {
                        None if !card.rank.is_king() => {
                            "Illegal move: only a King can start an empty column.".to_string()
                        }
                        Some(top) if top.color() == card.color() => {
                            "Illegal move: tableau piles need alternating colors.".to_string()
                        }
                        Some(top) if card.rank.value() + 1 != top.rank.value() => {
                            "Illegal move: tableau piles need one rank lower than the destination card."
                                .to_string()
                        }
                        _ => GENERIC.to_string(),
                    },
                    _ => GENERIC.to_string(),
                }
            }
            GameMode::FreeCell => {
                let moving = match sel.area {
                    CursorArea::Tableau => {
                        let pile = &self.freecell.tableau[sel.index];
                        let len = pile.len();
                        (len >= sel.count).then(|| pile[len - sel.count])
                    }
                    CursorArea::FreeCell => self.freecell.cell_card(sel.index),
                    CursorArea::Foundation => self.freecell.foundation_top(sel.index),
                    _ => None,
                };
                let Some(card) = moving else {
                    return GENERIC.to_string();
                };
                match cur.area {
                    CursorArea::Tableau => {
                        if sel.area == CursorArea::Tableau
                            && sel.count > self.freecell.max_movable(cur.index)
                        {
                            format!(
                                "Illegal move: not enough free cells/empty columns to move {} card(s) at once (max {} here).",
                                sel.count,
                                self.freecell.max_movable(cur.index)
                            )
                        } else {
                            match self.freecell.tableau_top(cur.index) {
                                Some(top) if top.color() == card.color() => {
                                    "Illegal move: tableau piles need alternating colors."
                                        .to_string()
                                }
                                Some(top) if card.rank.value() + 1 != top.rank.value() => {
                                    "Illegal move: tableau piles need one rank lower than the destination card."
                                        .to_string()
                                }
                                _ => GENERIC.to_string(),
                            }
                        }
                    }
                    CursorArea::Foundation => match self.freecell.foundation_top(cur.index) {
                        None if !card.rank.is_ace() => {
                            "Illegal move: foundations start with an Ace.".to_string()
                        }
                        Some(top) if top.suit != card.suit => {
                            "Illegal move: foundations only take the same suit, one rank up."
                                .to_string()
                        }
                        Some(top) if card.rank.value() != top.rank.value() + 1 => {
                            "Illegal move: foundations need the next rank up.".to_string()
                        }
                        _ => GENERIC.to_string(),
                    },
                    CursorArea::FreeCell => {
                        if self.freecell.cell_card(cur.index).is_some() {
                            "Illegal move: that free cell is already in use.".to_string()
                        } else {
                            GENERIC.to_string()
                        }
                    }
                    _ => GENERIC.to_string(),
                }
            }
            GameMode::SpiderMini => {
                if sel.area != CursorArea::Tableau || cur.area != CursorArea::Tableau {
                    return GENERIC.to_string();
                }
                let pile = &self.spider.tableau[sel.index];
                let len = pile.len();
                let Some(card) = (len >= sel.count).then(|| pile[len - sel.count]) else {
                    return GENERIC.to_string();
                };
                match self.spider.tableau_top(cur.index) {
                    Some(top) if card.rank.value() + 1 != top.rank.value() => {
                        "Illegal move: tableau piles need one rank lower than the destination card (any suit)."
                            .to_string()
                    }
                    _ => GENERIC.to_string(),
                }
            }
        }
    }

    fn pop_undo(&mut self) {
        match self.mode {
            GameMode::Klondike => {
                self.undo_klondike.pop_back();
            }
            GameMode::FreeCell => {
                self.undo_freecell.pop_back();
            }
            GameMode::SpiderMini => {
                self.undo_spider.pop_back();
            }
        }
    }

    fn place_klondike(&mut self, sel: Selected, cur: Cursor) -> bool {
        match cur.area {
            CursorArea::Foundation => match sel.area {
                CursorArea::Waste => self.klondike.move_waste_to_foundation(),
                CursorArea::Tableau => self.klondike.move_tableau_to_foundation(sel.index),
                _ => false,
            },
            CursorArea::Tableau => match sel.area {
                CursorArea::Waste => self.klondike.move_waste_to_tableau(cur.index),
                CursorArea::Foundation => self
                    .klondike
                    .move_foundation_to_tableau(sel.index, cur.index),
                CursorArea::Tableau => {
                    for count in (1..=sel.count).rev() {
                        if self
                            .klondike
                            .move_tableau_to_tableau(sel.index, cur.index, count)
                        {
                            return true;
                        }
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn place_freecell(&mut self, sel: Selected, cur: Cursor) -> bool {
        match cur.area {
            CursorArea::Tableau => match sel.area {
                CursorArea::Tableau => self
                    .freecell
                    .move_tableau_to_tableau(sel.index, sel.count, cur.index),
                CursorArea::FreeCell => self.freecell.move_cell_to_tableau(sel.index, cur.index),
                CursorArea::Foundation => self
                    .freecell
                    .move_foundation_to_tableau(sel.index, cur.index),
                _ => false,
            },
            CursorArea::Foundation => match sel.area {
                CursorArea::Tableau => self.freecell.move_tableau_to_foundation(sel.index),
                CursorArea::FreeCell => self.freecell.move_cell_to_foundation(sel.index),
                _ => false,
            },
            CursorArea::FreeCell => match sel.area {
                CursorArea::Tableau => self.freecell.move_tableau_top_to_cell(sel.index, cur.index),
                _ => false,
            },
            _ => false,
        }
    }

    fn place_spider(&mut self, sel: Selected, cur: Cursor) -> bool {
        if cur.area != CursorArea::Tableau || sel.area != CursorArea::Tableau {
            return false;
        }
        self.spider
            .move_tableau_to_tableau(sel.index, sel.count, cur.index)
    }

    // ---- legal-target highlighting ----

    /// Cursor targets where the current selection could legally go.
    /// Each entry is `(area, index)`.
    pub fn legal_targets(&self) -> Vec<(CursorArea, usize)> {
        let sel = match self.selected {
            Some(s) => s,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        match self.mode {
            GameMode::Klondike => {
                if sel.area == CursorArea::Tableau {
                    let len = self.klondike.tableau[sel.index].len();
                    if len >= sel.count {
                        let bottom = self.klondike.tableau[sel.index][len - sel.count];
                        for to in 0..7 {
                            if to != sel.index && self.klondike.can_place_on_tableau(bottom, to) {
                                out.push((CursorArea::Tableau, to));
                            }
                        }
                        // A multi-card grab can still send its top card up.
                        let top = self.klondike.tableau[sel.index][len - 1];
                        if (0..4).any(|f| self.klondike.can_place_on_foundation(top, f)) {
                            for f in 0..4 {
                                out.push((CursorArea::Foundation, f));
                            }
                        }
                    }
                } else {
                    let card = match sel.area {
                        CursorArea::Waste => self.klondike.waste_top(),
                        CursorArea::Foundation => self.klondike.foundation_top(sel.index),
                        _ => None,
                    };
                    if let Some(c) = card {
                        for to in 0..7 {
                            if self.klondike.can_place_on_tableau(c, to) {
                                out.push((CursorArea::Tableau, to));
                            }
                        }
                        if sel.area == CursorArea::Waste
                            && (0..4).any(|f| self.klondike.can_place_on_foundation(c, f))
                        {
                            for f in 0..4 {
                                out.push((CursorArea::Foundation, f));
                            }
                        }
                    }
                }
            }
            GameMode::FreeCell => {
                let top_card = match sel.area {
                    CursorArea::Tableau => self.freecell.tableau_top(sel.index),
                    CursorArea::FreeCell => self.freecell.cell_card(sel.index),
                    CursorArea::Foundation => self.freecell.foundation_top(sel.index),
                    _ => None,
                };
                if let Some(c) = top_card {
                    // Foundation targets (top single card).
                    let found_ok = (0..4).any(|f| self.freecell.can_place_on_foundation(c, f));
                    if found_ok && sel.area != CursorArea::Foundation {
                        for f in 0..4 {
                            out.push((CursorArea::Foundation, f));
                        }
                    }
                    // Tableau targets (bottom of the grabbed run).
                    let bottom = if sel.area == CursorArea::Tableau {
                        let len = self.freecell.tableau[sel.index].len();
                        if len >= sel.count {
                            Some(self.freecell.tableau[sel.index][len - sel.count])
                        } else {
                            None
                        }
                    } else {
                        Some(c)
                    };
                    if let Some(b) = bottom {
                        for to in 0..8 {
                            if sel.area == CursorArea::Tableau && to == sel.index {
                                continue;
                            }
                            // Only suggest tableau targets the run fits into.
                            if sel.area == CursorArea::Tableau
                                && sel.count > self.freecell.max_movable(to)
                            {
                                continue;
                            }
                            if self.freecell.can_place_on_tableau(b, to) {
                                out.push((CursorArea::Tableau, to));
                            }
                        }
                    }
                    // Empty-cell targets for single tableau tops.
                    if sel.area == CursorArea::Tableau && self.freecell.free_count() > 0 {
                        for cell in 0..self.freecell.num_cells {
                            if self.freecell.cell_card(cell).is_none() {
                                out.push((CursorArea::FreeCell, cell));
                            }
                        }
                    }
                }
            }
            GameMode::SpiderMini => {
                if sel.area == CursorArea::Tableau {
                    let run = self.spider.movable_run_len(sel.index);
                    let len = self.spider.tableau[sel.index].len();
                    for count in (1..=sel.count.min(run).min(len)).rev() {
                        let bottom = self.spider.tableau[sel.index][len - count];
                        let mut any = false;
                        for to in 0..10 {
                            if to != sel.index && self.spider.can_place_on_tableau(bottom, to) {
                                out.push((CursorArea::Tableau, to));
                                any = true;
                            }
                        }
                        if any {
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

pub fn difficulty_label(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => "Easy",
        Difficulty::Normal => "Normal",
        Difficulty::Hard => "Hard",
    }
}

fn difficulty_menu_index(d: Difficulty) -> usize {
    match d {
        Difficulty::Easy => 0,
        Difficulty::Normal => 1,
        Difficulty::Hard => 2,
    }
}

fn difficulty_from_menu_index(i: usize) -> Difficulty {
    match i {
        0 => Difficulty::Easy,
        2 => Difficulty::Hard,
        _ => Difficulty::Normal,
    }
}

fn mode_menu_index(m: GameMode) -> usize {
    match m {
        GameMode::Klondike => 0,
        GameMode::FreeCell => 1,
        GameMode::SpiderMini => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_stays_in_bounds_per_mode() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.switch_mode(GameMode::SpiderMini);
        app.jump_to_tableau(9);
        assert_eq!(app.cursor.index, 9);
        app.jump_to_tableau(10); // out of range, ignored
        assert_eq!(app.cursor.index, 9);
        app.switch_mode(GameMode::Klondike);
        assert!(app.cursor.index < 7);
    }

    #[test]
    fn undo_restores_freecell_move() {
        let mut app = App::new(DrawMode::Draw1, Difficulty::Easy);
        app.switch_mode(GameMode::FreeCell);
        // Force a known layout: single ace on tableau 0.
        for p in app.freecell.tableau.iter_mut() {
            p.clear();
        }
        for f in app.freecell.foundations.iter_mut() {
            f.clear();
        }
        app.freecell.freecells = [None, None, None, None, None, None];
        app.freecell.tableau[0].push(crate::cards::Card::new_face_up(
            crate::cards::Suit::Spades,
            crate::cards::Rank::ACE,
        ));
        app.cursor = Cursor {
            area: CursorArea::Tableau,
            index: 0,
        };
        app.select_or_place(); // grab
        app.cursor = Cursor {
            area: CursorArea::Foundation,
            index: 0,
        };
        app.select_or_place(); // place ace on foundation
        assert_eq!(app.freecell.foundations[0].len(), 1);
        app.undo();
        assert_eq!(app.freecell.foundations[0].len(), 0);
        assert_eq!(app.freecell.tableau[0].len(), 1);
    }

    #[test]
    fn klondike_grab_and_drop_four_clubs_on_five_diamonds() {
        // Reported flow: Space grabs 4♣ from one tableau pile, Space
        // drops it onto a pile topped by 5♦. Must succeed, not report
        // "Illegal move".
        let mut app = App::new(DrawMode::Draw1, Difficulty::Easy);
        for p in app.klondike.tableau.iter_mut() {
            p.clear();
        }
        app.klondike.stock.clear();
        app.klondike.waste.clear();
        app.klondike.tableau[0].push(crate::cards::Card::new_face_up(
            crate::cards::Suit::Diamonds,
            crate::cards::Rank(5),
        ));
        app.klondike.tableau[1].push(crate::cards::Card::new_face_up(
            crate::cards::Suit::Clubs,
            crate::cards::Rank(4),
        ));
        app.mode = GameMode::Klondike;
        app.cursor = Cursor {
            area: CursorArea::Tableau,
            index: 1,
        };
        app.select_or_place(); // grab
        assert!(app.selected.is_some(), "4♣ should be grabbable");
        app.cursor = Cursor {
            area: CursorArea::Tableau,
            index: 0,
        };
        app.select_or_place(); // drop onto 5♦
        assert_eq!(app.klondike.tableau[0].len(), 2);
        assert!(app.selected.is_none());
        assert!(
            !app.status_message.contains("Illegal"),
            "status was: {}",
            app.status_message
        );
    }

    #[test]
    fn klondike_same_color_drop_stays_illegal() {
        // Guard the other side: 4♠ onto 5♣ (same color) must still be
        // refused and leave both piles untouched.
        let mut app = App::new(DrawMode::Draw1, Difficulty::Easy);
        for p in app.klondike.tableau.iter_mut() {
            p.clear();
        }
        app.klondike.stock.clear();
        app.klondike.waste.clear();
        app.klondike.tableau[0].push(crate::cards::Card::new_face_up(
            crate::cards::Suit::Clubs,
            crate::cards::Rank(5),
        ));
        app.klondike.tableau[1].push(crate::cards::Card::new_face_up(
            crate::cards::Suit::Spades,
            crate::cards::Rank(4),
        ));
        app.mode = GameMode::Klondike;
        app.cursor = Cursor {
            area: CursorArea::Tableau,
            index: 1,
        };
        app.select_or_place(); // grab
        app.cursor = Cursor {
            area: CursorArea::Tableau,
            index: 0,
        };
        app.select_or_place(); // drop: illegal
        assert_eq!(app.klondike.tableau[0].len(), 1);
        assert_eq!(app.klondike.tableau[1].len(), 1);
        assert!(app.status_message.contains("Illegal"));
    }

    #[test]
    fn menu_confirm_applies_spider_difficulty() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.open_menu();
        app.menu_index = 2; // spider
        app.menu_diff_index = 0; // easy
        app.menu_confirm();
        assert_eq!(app.mode, GameMode::SpiderMini);
        assert_eq!(app.difficulty, Difficulty::Easy);
        assert_eq!(app.spider.suits, SpiderSuits::One);
    }

    #[test]
    fn hard_mode_exhausted_draw_keeps_undo_snapshot() {
        // Regression: a draw that exhausts Hard-mode redeals mutates
        // klondike.game_over even though draw_from_stock() returns false.
        // The pre-draw undo snapshot must survive so `u` restores the state
        // right before this draw, not the one before that.
        let mut app = App::new(DrawMode::Draw3, Difficulty::Hard);
        app.mode = GameMode::Klondike;
        while !app.klondike.stock.is_empty() {
            app.draw();
        }
        for _ in 0..3 {
            app.draw(); // redeal
            while !app.klondike.stock.is_empty() {
                app.draw();
            }
        }
        assert!(!app.klondike.can_redeal());

        let depth_before = app.undo_depth();
        let waste_before = app.klondike.waste.clone();
        app.draw(); // 4th redeal attempt: refused, sets game_over
        assert!(app.klondike.is_game_over());
        assert_eq!(
            app.undo_depth(),
            depth_before + 1,
            "the pre-draw snapshot must be kept, not discarded"
        );

        app.undo();
        assert!(!app.klondike.is_game_over());
        assert_eq!(app.klondike.waste, waste_before);
    }

    #[test]
    fn quit_new_game_and_restart_require_confirmation() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.screen = Screen::Game;

        app.request_quit();
        assert!(!app.should_quit, "quit must wait for confirmation");
        assert_eq!(app.confirm, Some(PendingConfirm::Quit));
        app.cancel_pending();
        assert!(!app.should_quit);
        assert!(app.confirm.is_none());

        app.request_quit();
        app.confirm_pending();
        assert!(app.should_quit);

        let before = app.klondike.tableau.clone();
        app.request_new_game();
        assert_eq!(app.klondike.tableau, before, "cancel must not touch state");
        app.cancel_pending();
        assert_eq!(app.klondike.tableau, before);

        app.request_restart();
        app.confirm_pending();
        assert!(app.confirm.is_none());
    }
}
