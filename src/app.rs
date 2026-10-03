//! Application state: one session per game, the menu, cursor and selection,
//! mouse hit regions, undo, hints, and per-game timers.

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use crate::cards::{Card, random_deal};
use crate::freecell::FreeCellGame;
use crate::klondike::{DrawMode, KlondikeGame};
use crate::rules::{Difficulty, MoveError, Solitaire, cards, check_foundation};
use crate::spider::{SpiderGame, SpiderSuits};

/// Which solitaire variant is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Klondike,
    FreeCell,
    Spider,
}

impl GameMode {
    pub const ALL: [GameMode; 3] = [GameMode::Klondike, GameMode::FreeCell, GameMode::Spider];

    pub fn index(self) -> usize {
        match self {
            GameMode::Klondike => 0,
            GameMode::FreeCell => 1,
            GameMode::Spider => 2,
        }
    }
}

impl fmt::Display for GameMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GameMode::Klondike => "Klondike",
            GameMode::FreeCell => "FreeCell",
            GameMode::Spider => "Spider",
        })
    }
}

/// Which screen is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    Game,
    #[default]
    Menu,
}

/// Cursor pile area. Not every area exists in every game (see
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

/// A grabbed pile ready to place elsewhere. `count` cards are held out of a
/// grabbable run of `max`. When `explicit` is false the player didn't pick
/// a count, so placing may move fewer cards if only a shorter run fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    pub area: CursorArea,
    pub index: usize,
    pub count: usize,
    pub max: usize,
    pub explicit: bool,
}

impl Selected {
    /// Counts to try when placing, largest first.
    fn counts(self) -> Vec<usize> {
        if self.explicit {
            vec![self.count]
        } else {
            (1..=self.count).rev().collect()
        }
    }
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

/// Settings for a fresh deal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DealRequest {
    pub mode: GameMode,
    pub difficulty: Difficulty,
    pub draw_mode: DrawMode,
}

/// A destructive action (discards the current game or exits) awaiting a
/// second keypress to confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingConfirm {
    Quit,
    NewGame,
    Restart,
    Deal(DealRequest),
}

/// Commands shared by the keyboard, the toolbar and dialog buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    New,
    Restart,
    Undo,
    Draw,
    Auto,
    Hint,
    Menu,
    Help,
    Quit,
}

/// What a screen region does when clicked. `card` is the index of the
/// clicked card inside a tableau pile, when a specific card was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Pile {
        area: CursorArea,
        index: usize,
        card: Option<usize>,
    },
    Action(Action),
    MenuGame(GameMode),
    MenuDifficulty(Difficulty),
    MenuDraw,
    MenuStart,
}

/// A clickable screen rectangle, recorded by the renderer each frame.
/// Later regions sit on top of earlier ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRegion {
    pub rect: Rect,
    pub hit: Hit,
}

/// A pausable stopwatch.
#[derive(Debug, Clone, Default)]
pub struct Clock {
    accumulated: Duration,
    since: Option<Instant>,
}

impl Clock {
    pub fn elapsed(&self) -> Duration {
        self.accumulated + self.since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    pub fn set_running(&mut self, running: bool) {
        match (running, self.since) {
            (true, None) => self.since = Some(Instant::now()),
            (false, Some(t)) => {
                self.accumulated += t.elapsed();
                self.since = None;
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// One game plus everything that belongs to it: its difficulty, undo
/// history and play clock.
#[derive(Debug)]
pub struct Session<G> {
    pub game: G,
    pub difficulty: Difficulty,
    pub clock: Clock,
    undo: VecDeque<G>,
}

impl<G: Solitaire + Clone> Session<G> {
    fn new(game: G, difficulty: Difficulty) -> Self {
        Self {
            game,
            difficulty,
            clock: Clock::default(),
            undo: VecDeque::new(),
        }
    }

    /// Run a move; when it succeeds, the pre-move state becomes one undo
    /// step. Failed moves must leave the game untouched.
    pub fn apply(&mut self, f: impl FnOnce(&mut G) -> bool) -> bool {
        let before = self.game.clone();
        let ok = f(&mut self.game);
        if ok {
            if self.undo.len() >= MAX_UNDO {
                self.undo.pop_front();
            }
            self.undo.push_back(before);
        }
        ok
    }

    pub fn undo(&mut self) -> bool {
        self.undo.pop_back().map(|g| self.game = g).is_some()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Started and not finished: discarding it loses progress.
    pub fn in_progress(&self) -> bool {
        self.game.move_count() > 0 && !self.game.is_won()
    }

    fn replace(&mut self, game: G, difficulty: Difficulty) {
        *self = Self::new(game, difficulty);
    }

    fn restart(&mut self, restart: impl FnOnce(&mut G)) {
        restart(&mut self.game);
        self.undo.clear();
        self.clock.reset();
    }
}

/// Evaluate `$body` with `$s` bound to the active session.
macro_rules! with_session {
    ($app:expr, $s:ident => $body:expr) => {
        match $app.mode {
            GameMode::Klondike => {
                let $s = &$app.klondike;
                $body
            }
            GameMode::FreeCell => {
                let $s = &$app.freecell;
                $body
            }
            GameMode::Spider => {
                let $s = &$app.spider;
                $body
            }
        }
    };
}

/// Like `with_session!`, but the session is borrowed mutably.
macro_rules! with_session_mut {
    ($app:expr, $s:ident => $body:expr) => {
        match $app.mode {
            GameMode::Klondike => {
                let $s = &mut $app.klondike;
                $body
            }
            GameMode::FreeCell => {
                let $s = &mut $app.freecell;
                $body
            }
            GameMode::Spider => {
                let $s = &mut $app.spider;
                $body
            }
        }
    };
}

/// Menu selections (applied on confirm).
#[derive(Debug, Clone, Copy)]
pub struct MenuState {
    pub game: GameMode,
    pub difficulty: Difficulty,
}

/// Top-level app state.
#[derive(Debug)]
pub struct App {
    pub mode: GameMode,
    pub screen: Screen,
    pub klondike: Session<KlondikeGame>,
    pub freecell: Session<FreeCellGame>,
    pub spider: Session<SpiderGame>,
    /// Klondike draw count chosen with `--draw` or in the menu; `None`
    /// follows the difficulty (Easy draws 1, others 3).
    pub draw_override: Option<DrawMode>,
    pub cursor: Cursor,
    pub selected: Option<Selected>,
    pub menu: MenuState,
    pub show_help: bool,
    pub status_message: String,
    pub should_quit: bool,
    pub confirm: Option<PendingConfirm>,
    /// Whether the terminal renders 24-bit color (else the UI downsamples).
    pub truecolor: bool,
    /// Clickable regions from the last rendered frame.
    pub hits: Vec<HitRegion>,
}

const MENU_HELP: &str = "Pick a game and difficulty, then Enter (or click Start).";

impl App {
    /// `deal` fixes the first deal of every game (`--deal`); otherwise each
    /// starts on a random deal.
    pub fn new(difficulty: Difficulty, draw_override: Option<DrawMode>, deal: Option<u32>) -> Self {
        let draw_mode = draw_override.unwrap_or(DrawMode::for_difficulty(difficulty));
        let deal_or_random = || deal.unwrap_or_else(random_deal);
        Self {
            mode: GameMode::Klondike,
            screen: Screen::Menu,
            klondike: Session::new(make_klondike(draw_mode, difficulty, deal), difficulty),
            freecell: Session::new(FreeCellGame::new(difficulty, deal_or_random()), difficulty),
            spider: Session::new(
                SpiderGame::new(SpiderSuits::from_difficulty(difficulty), deal_or_random()),
                difficulty,
            ),
            draw_override,
            cursor: Cursor {
                area: CursorArea::Tableau,
                index: 0,
            },
            selected: None,
            menu: MenuState {
                game: GameMode::Klondike,
                difficulty,
            },
            show_help: false,
            status_message: MENU_HELP.to_string(),
            should_quit: false,
            confirm: None,
            truecolor: true,
            hits: Vec::new(),
        }
    }

    // ---- active-game accessors ----

    pub fn game(&self) -> &dyn Solitaire {
        with_session!(self, s => &s.game)
    }

    pub fn difficulty(&self) -> Difficulty {
        with_session!(self, s => s.difficulty)
    }

    pub fn undo_depth(&self) -> usize {
        with_session!(self, s => s.undo_depth())
    }

    pub fn elapsed(&self) -> Duration {
        with_session!(self, s => s.clock.elapsed())
    }

    fn in_progress(&self, mode: GameMode) -> bool {
        match mode {
            GameMode::Klondike => self.klondike.in_progress(),
            GameMode::FreeCell => self.freecell.in_progress(),
            GameMode::Spider => self.spider.in_progress(),
        }
    }

    pub fn is_won(&self) -> bool {
        self.game().is_won()
    }

    pub fn is_stuck(&self) -> bool {
        self.game().is_stuck()
    }

    /// Run the active game's clock only while it is actually being played:
    /// on the table, help closed, not yet won. Call once per event loop.
    pub fn tick(&mut self) {
        let playing = self.screen == Screen::Game && !self.show_help && !self.is_won();
        self.klondike
            .clock
            .set_running(playing && self.mode == GameMode::Klondike);
        self.freecell
            .clock
            .set_running(playing && self.mode == GameMode::FreeCell);
        self.spider
            .clock
            .set_running(playing && self.mode == GameMode::Spider);
    }

    // ---- commands ----

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::New => self.request(PendingConfirm::NewGame),
            Action::Restart => self.request(PendingConfirm::Restart),
            Action::Undo => self.undo(),
            Action::Draw => self.draw(),
            Action::Auto => self.auto_foundation(),
            Action::Hint => self.hint(),
            Action::Menu => self.open_menu(),
            Action::Help => self.show_help = !self.show_help,
            Action::Quit => self.request(PendingConfirm::Quit),
        }
    }

    // ---- destructive-action confirmation ----

    /// Ask before an action that would lose progress; act at once when
    /// there is nothing to lose.
    pub fn request(&mut self, action: PendingConfirm) {
        let needs_confirm = match action {
            PendingConfirm::Quit | PendingConfirm::NewGame | PendingConfirm::Restart => {
                self.in_progress(self.mode)
            }
            PendingConfirm::Deal(req) => self.in_progress(req.mode),
        };
        if !needs_confirm {
            self.confirm = Some(action);
            self.confirm_pending();
            return;
        }
        self.confirm = Some(action);
        let question = match action {
            PendingConfirm::Quit => "Quit Solitaire?".to_string(),
            PendingConfirm::NewGame => "Discard this game and deal a new one?".to_string(),
            PendingConfirm::Restart => "Restart this deal from the beginning?".to_string(),
            PendingConfirm::Deal(req) => {
                format!("Discard your {} game and deal a new one?", req.mode)
            }
        };
        self.status_message = format!("{question} (y to confirm, any other key to cancel)");
    }

    /// y/Enter on a pending confirmation: carry out the action.
    pub fn confirm_pending(&mut self) {
        match self.confirm.take() {
            Some(PendingConfirm::Quit) => self.should_quit = true,
            Some(PendingConfirm::NewGame) => self.new_game(),
            Some(PendingConfirm::Restart) => self.restart(),
            Some(PendingConfirm::Deal(req)) => self.deal(req),
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

    pub fn open_menu(&mut self) {
        self.screen = Screen::Menu;
        self.selected = None;
        self.menu = MenuState {
            game: self.mode,
            difficulty: self.difficulty(),
        };
        self.status_message = MENU_HELP.to_string();
    }

    pub fn close_menu(&mut self) {
        self.screen = Screen::Game;
        self.status_message = format!("{} · {}.", self.mode, self.difficulty());
    }

    pub fn menu_move(&mut self, dir: Dir) {
        let i = self.menu.game.index();
        match dir {
            Dir::Up => self.menu.game = GameMode::ALL[i.saturating_sub(1)],
            Dir::Down => self.menu.game = GameMode::ALL[(i + 1).min(GameMode::ALL.len() - 1)],
            Dir::Left => self.menu.difficulty = self.menu.difficulty.easier(),
            Dir::Right => self.menu.difficulty = self.menu.difficulty.harder(),
        }
    }

    /// Klondike draw count the menu would deal with.
    pub fn menu_draw_mode(&self) -> DrawMode {
        self.draw_override
            .unwrap_or(DrawMode::for_difficulty(self.menu.difficulty))
    }

    pub fn menu_toggle_draw(&mut self) {
        self.draw_override = Some(self.menu_draw_mode().toggled());
    }

    fn menu_request(&self) -> DealRequest {
        DealRequest {
            mode: self.menu.game,
            difficulty: self.menu.difficulty,
            draw_mode: self.menu_draw_mode(),
        }
    }

    /// Whether confirming the menu returns to an unfinished game with the
    /// same settings (instead of dealing a new one).
    pub fn menu_resumes(&self) -> bool {
        let req = self.menu_request();
        match req.mode {
            GameMode::Klondike => {
                self.klondike.difficulty == req.difficulty
                    && self.klondike.game.draw_mode == req.draw_mode
                    && !self.klondike.game.is_won()
            }
            GameMode::FreeCell => {
                self.freecell.difficulty == req.difficulty && !self.freecell.game.is_won()
            }
            GameMode::Spider => {
                self.spider.difficulty == req.difficulty && !self.spider.game.is_won()
            }
        }
    }

    /// Enter on the menu: resume the chosen game if its settings match,
    /// otherwise deal a new one (asking first if that discards progress).
    pub fn menu_confirm(&mut self) {
        let req = self.menu_request();
        if self.menu_resumes() {
            self.mode = req.mode;
            self.selected = None;
            self.ensure_cursor_valid();
            self.close_menu();
        } else {
            self.request(PendingConfirm::Deal(req));
        }
    }

    // ---- dealing ----

    /// Fresh deal with explicit settings; switches to that game.
    fn deal(&mut self, req: DealRequest) {
        let deal = random_deal();
        match req.mode {
            GameMode::Klondike => self.klondike.replace(
                make_klondike(req.draw_mode, req.difficulty, None),
                req.difficulty,
            ),
            GameMode::FreeCell => self
                .freecell
                .replace(FreeCellGame::new(req.difficulty, deal), req.difficulty),
            GameMode::Spider => self.spider.replace(
                SpiderGame::new(SpiderSuits::from_difficulty(req.difficulty), deal),
                req.difficulty,
            ),
        }
        self.mode = req.mode;
        self.screen = Screen::Game;
        self.selected = None;
        self.ensure_cursor_valid();
        self.status_message = format!(
            "New {} deal #{} · {}.",
            self.mode,
            self.game().deal(),
            self.difficulty()
        );
    }

    /// New random deal of the current game with its current settings.
    fn new_game(&mut self) {
        self.deal(DealRequest {
            mode: self.mode,
            difficulty: self.difficulty(),
            draw_mode: self.klondike.game.draw_mode,
        });
    }

    /// Replay the current deal from the start.
    fn restart(&mut self) {
        match self.mode {
            GameMode::Klondike => self.klondike.restart(KlondikeGame::restart),
            GameMode::FreeCell => self.freecell.restart(FreeCellGame::restart),
            GameMode::Spider => self.spider.restart(SpiderGame::restart),
        }
        self.selected = None;
        self.status_message = format!("Restarted deal #{}.", self.game().deal());
    }

    // ---- undo ----

    pub fn undo(&mut self) {
        let restored = with_session_mut!(self, s => s.undo());
        self.selected = None;
        self.status_message = if restored {
            "Undone.".to_string()
        } else {
            "Nothing to undo.".to_string()
        };
    }

    // ---- draw / auto / hint ----

    /// Draw (Klondike) or deal a row (Spider).
    pub fn draw(&mut self) {
        self.selected = None;
        match self.mode {
            GameMode::Klondike => {
                let recycling = self.klondike.game.stock.is_empty();
                if self.klondike.apply(KlondikeGame::draw_from_stock) {
                    self.status_message = if recycling {
                        "Recycled the waste into the stock.".to_string()
                    } else {
                        "Drew from the stock.".to_string()
                    };
                    self.after_move();
                } else {
                    let g = &self.klondike.game;
                    self.status_message = if g.is_won() {
                        "Already won!".to_string()
                    } else if g.waste.is_empty() {
                        "Stock and waste are both empty.".to_string()
                    } else {
                        "No redeals left on Hard.".to_string()
                    };
                }
            }
            GameMode::Spider => {
                if self.spider.game.stock.is_empty() {
                    self.status_message = "The stock is empty.".to_string();
                } else if self.spider.apply(SpiderGame::deal_from_stock) {
                    self.status_message = match self.spider.game.stock_deals_remaining() {
                        1 => "Dealt a row. 1 deal left.".to_string(),
                        n => format!("Dealt a row. {n} deals left."),
                    };
                    self.after_move();
                } else {
                    self.status_message =
                        "Fill every empty column before dealing from the stock.".to_string();
                }
            }
            GameMode::FreeCell => {
                self.status_message = "FreeCell has no stock.".to_string();
            }
        }
    }

    /// `a`: send safe cards up, or finish the game once nothing is hidden.
    pub fn auto_foundation(&mut self) {
        self.selected = None;
        let mut moved = 0;
        let ok = match self.mode {
            GameMode::Klondike => self.klondike.apply(|g| {
                moved = g.auto_finish() + g.auto_move_safe();
                moved > 0
            }),
            GameMode::FreeCell => self.freecell.apply(|g| {
                moved = g.auto_finish() + g.auto_move_safe();
                moved > 0
            }),
            GameMode::Spider => {
                self.status_message = "Completed runs are removed automatically.".to_string();
                return;
            }
        };
        if ok {
            self.status_message = format!("Moved {} to the foundations.", cards(moved));
            self.after_move();
        } else {
            self.status_message = "No cards are safe to move up right now.".to_string();
        }
    }

    /// `F` / right-click: send the top card of the cursor pile up.
    pub fn send_to_foundation(&mut self) {
        self.selected = None;
        let Cursor { area, index } = self.cursor;
        let ok = match (self.mode, area) {
            (GameMode::Klondike, CursorArea::Tableau) => {
                self.klondike.apply(|g| g.move_tableau_to_foundation(index))
            }
            (GameMode::Klondike, CursorArea::Waste) => {
                self.klondike.apply(KlondikeGame::move_waste_to_foundation)
            }
            (GameMode::FreeCell, CursorArea::Tableau) => self.freecell.apply(|g| {
                let ok = g.move_tableau_to_foundation(index);
                if ok {
                    g.auto_move_safe();
                }
                ok
            }),
            (GameMode::FreeCell, CursorArea::FreeCell) => self.freecell.apply(|g| {
                let ok = g.move_cell_to_foundation(index);
                if ok {
                    g.auto_move_safe();
                }
                ok
            }),
            _ => false,
        };
        if ok {
            self.status_message = "Moved to a foundation.".to_string();
            self.after_move();
        } else {
            self.status_message = "That card can't go to a foundation yet.".to_string();
        }
    }

    pub fn hint(&mut self) {
        self.status_message = self
            .game()
            .hint()
            .unwrap_or_else(|| "No useful moves left. u undo · r restart · n new deal".to_string());
    }

    /// Status follow-up after any successful move.
    fn after_move(&mut self) {
        if self.is_won() {
            let secs = self.elapsed().as_secs();
            self.status_message = format!(
                "You win! Score {} in {} moves ({:02}:{:02}). n new deal · m menu",
                self.game().score(),
                self.game().move_count(),
                secs / 60,
                secs % 60
            );
        } else if self.is_stuck() {
            self.status_message =
                "No useful moves left. u undo · r restart · n new deal".to_string();
        } else if self.can_auto_finish() {
            self.status_message = "Everything is in order: press a to finish.".to_string();
        }
    }

    fn can_auto_finish(&self) -> bool {
        match self.mode {
            GameMode::Klondike => self.klondike.game.can_auto_finish(),
            GameMode::FreeCell => self.freecell.game.can_auto_finish(),
            GameMode::Spider => false,
        }
    }

    // ---- cursor navigation ----

    /// Pile areas in Tab order for the active game.
    fn tab_areas(&self) -> &'static [CursorArea] {
        match self.mode {
            GameMode::Klondike => &[
                CursorArea::Stock,
                CursorArea::Waste,
                CursorArea::Foundation,
                CursorArea::Tableau,
            ],
            GameMode::FreeCell => &[
                CursorArea::FreeCell,
                CursorArea::Foundation,
                CursorArea::Tableau,
            ],
            GameMode::Spider => &[CursorArea::Stock, CursorArea::Tableau],
        }
    }

    /// Number of piles in an area for the active game.
    pub fn area_size(&self, area: CursorArea) -> usize {
        match (self.mode, area) {
            (GameMode::Klondike, CursorArea::Tableau) => crate::klondike::TABLEAU_PILES,
            (GameMode::FreeCell, CursorArea::Tableau) => crate::freecell::TABLEAU_PILES,
            (GameMode::Spider, CursorArea::Tableau) => crate::spider::TABLEAU_PILES,
            (GameMode::Klondike | GameMode::FreeCell, CursorArea::Foundation) => 4,
            (GameMode::Klondike, CursorArea::Stock | CursorArea::Waste)
            | (GameMode::Spider, CursorArea::Stock) => 1,
            (GameMode::FreeCell, CursorArea::FreeCell) => self.freecell.game.num_cells,
            _ => 0,
        }
    }

    /// Piles in the top row, left to right.
    fn top_row(&self) -> Vec<Cursor> {
        let areas: &[CursorArea] = match self.mode {
            GameMode::Klondike => &[CursorArea::Stock, CursorArea::Waste, CursorArea::Foundation],
            GameMode::FreeCell => &[CursorArea::FreeCell, CursorArea::Foundation],
            GameMode::Spider => &[CursorArea::Stock],
        };
        areas
            .iter()
            .flat_map(|&area| (0..self.area_size(area)).map(move |index| Cursor { area, index }))
            .collect()
    }

    pub fn ensure_cursor_valid(&mut self) {
        if self.area_size(self.cursor.area) == 0 {
            self.cursor = Cursor {
                area: CursorArea::Tableau,
                index: 0,
            };
        }
        let size = self.area_size(self.cursor.area);
        self.cursor.index = self.cursor.index.min(size - 1);
    }

    /// Arrow keys: Left/Right walk along the current row (crossing areas),
    /// Up/Down jump between the top row and the tableau, landing on the
    /// pile in roughly the same screen column.
    pub fn move_cursor(&mut self, dir: Dir) {
        self.ensure_cursor_valid();
        let top = self.top_row();
        let tableau = self.area_size(CursorArea::Tableau);
        let on_tableau = self.cursor.area == CursorArea::Tableau;
        let pos = if on_tableau {
            self.cursor.index
        } else {
            top.iter().position(|&c| c == self.cursor).unwrap_or(0)
        };
        let row_len = if on_tableau { tableau } else { top.len() };
        let target = match dir {
            Dir::Left => pos.saturating_sub(1),
            Dir::Right => (pos + 1).min(row_len - 1),
            Dir::Up if on_tableau => {
                self.cursor = top[scale_index(pos, tableau, top.len())];
                return;
            }
            Dir::Down if !on_tableau => {
                self.cursor = Cursor {
                    area: CursorArea::Tableau,
                    index: scale_index(pos, top.len(), tableau),
                };
                return;
            }
            Dir::Up | Dir::Down => return,
        };
        if on_tableau {
            self.cursor.index = target;
        } else {
            self.cursor = top[target];
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
        self.cursor = Cursor {
            area: areas[next],
            index: 0,
        };
    }

    pub fn tab_next(&mut self) {
        self.tab_area_step(false);
    }

    pub fn tab_prev(&mut self) {
        self.tab_area_step(true);
    }

    /// Put the cursor on `area`'s pile `index`, if that pile exists.
    pub fn focus(&mut self, area: CursorArea, index: usize) {
        if index < self.area_size(area) {
            self.cursor = Cursor { area, index };
        }
    }

    pub fn cancel_selection(&mut self) {
        self.selected = None;
        self.status_message = "Selection cancelled.".to_string();
    }

    // ---- select / place ----

    /// Space/Enter: draw on the stock, grab from the cursor pile, or place
    /// the grabbed cards onto the cursor pile.
    pub fn select_or_place(&mut self) {
        if self.is_won() {
            self.status_message = "Already won! Press n for a new deal.".to_string();
            return;
        }
        if self.cursor.area == CursorArea::Stock {
            self.draw();
            return;
        }
        match self.selected {
            None => self.grab(None),
            Some(sel) if sel.area == self.cursor.area && sel.index == self.cursor.index => {
                self.cancel_selection();
            }
            Some(sel) => self.place(sel),
        }
    }

    /// Cards that may be picked up from a pile (0 = nothing).
    fn grabbable(&self, area: CursorArea, index: usize) -> usize {
        let single = |c: Option<Card>| usize::from(c.is_some());
        match (self.mode, area) {
            (GameMode::Klondike, CursorArea::Tableau) => {
                self.klondike.game.grab_count_tableau(index)
            }
            (GameMode::Klondike, CursorArea::Waste) => single(self.klondike.game.waste_top()),
            (GameMode::Klondike, CursorArea::Foundation) => {
                single(self.klondike.game.foundation_top(index))
            }
            (GameMode::FreeCell, CursorArea::Tableau) => {
                self.freecell.game.grab_count_tableau(index)
            }
            (GameMode::FreeCell, CursorArea::FreeCell) => {
                single(self.freecell.game.cell_card(index))
            }
            (GameMode::FreeCell, CursorArea::Foundation) => {
                single(self.freecell.game.foundation_top(index))
            }
            (GameMode::Spider, CursorArea::Tableau) => self.spider.game.movable_run_len(index),
            _ => 0,
        }
    }

    fn tableau_len(&self, index: usize) -> usize {
        match self.mode {
            GameMode::Klondike => self.klondike.game.tableau[index].len(),
            GameMode::FreeCell => self.freecell.game.tableau[index].len(),
            GameMode::Spider => self.spider.game.tableau[index].len(),
        }
    }

    /// Grab from the cursor pile: the whole movable run, or (for a clicked
    /// tableau card) the run starting at that card.
    fn grab(&mut self, from_card: Option<usize>) {
        let Cursor { area, index } = self.cursor;
        let max = self.grabbable(area, index);
        if max == 0 {
            self.status_message = "Nothing to grab there.".to_string();
            return;
        }
        let picked = from_card
            .filter(|_| area == CursorArea::Tableau)
            .map(|card| self.tableau_len(index).saturating_sub(card))
            .filter(|&n| (1..=max).contains(&n));
        self.selected = Some(Selected {
            area,
            index,
            count: picked.unwrap_or(max),
            max,
            explicit: picked.is_some(),
        });
        self.describe_grab();
    }

    fn describe_grab(&mut self) {
        if let Some(sel) = self.selected {
            let adjust = if sel.max > 1 {
                " · +/- change count"
            } else {
                ""
            };
            self.status_message = format!(
                "Holding {}. Space on a green pile to place{adjust} · Esc cancel.",
                cards(sel.count)
            );
        }
    }

    /// `+` / `-`: hold more or fewer cards of the grabbed run.
    pub fn adjust_grab(&mut self, more: bool) {
        if let Some(sel) = &mut self.selected {
            sel.count = if more {
                (sel.count + 1).min(sel.max)
            } else {
                sel.count.saturating_sub(1).max(1)
            };
            sel.explicit = true;
            self.describe_grab();
        }
    }

    /// Place a grabbed pile onto the cursor pile.
    fn place(&mut self, sel: Selected) {
        let cur = self.cursor;
        let counts = sel.counts();
        let ok = match self.mode {
            GameMode::Klondike => self
                .klondike
                .apply(|g| place_klondike(g, sel, cur, &counts)),
            GameMode::FreeCell => self.freecell.apply(|g| {
                let ok = place_freecell(g, sel, cur, &counts);
                // Auto-play safe cards, except right after pulling a card
                // down from a foundation (it would just go straight back).
                if ok && sel.area != CursorArea::Foundation {
                    g.auto_move_safe();
                }
                ok
            }),
            GameMode::Spider => self.spider.apply(|g| {
                cur.area == CursorArea::Tableau
                    && sel.area == CursorArea::Tableau
                    && counts
                        .iter()
                        .any(|&n| g.move_tableau_to_tableau(sel.index, cur.index, n))
            }),
        };
        if ok {
            self.selected = None;
            self.status_message = "Moved.".to_string();
            self.after_move();
        } else {
            // Keep the grab so the player can try another target.
            self.status_message = self.move_error(sel, cur).to_string();
        }
    }

    /// Why placing `sel` onto `cur` is refused.
    fn move_error(&self, sel: Selected, cur: Cursor) -> MoveError {
        use CursorArea::{Foundation, FreeCell, Tableau, Waste};
        let err = |r: Result<(), MoveError>| r.err().unwrap_or(MoveError::NotAllowed);
        match self.mode {
            GameMode::Klondike => {
                let g = &self.klondike.game;
                let top = match sel.area {
                    Waste => g.waste_top(),
                    Foundation => g.foundation_top(sel.index),
                    Tableau => g.tableau_top(sel.index),
                    _ => None,
                };
                match (sel.area, cur.area, top) {
                    (Tableau, Tableau, _) => {
                        err(g.check_tableau_move(sel.index, cur.index, sel.count))
                    }
                    (Waste | Foundation, Tableau, Some(c)) => err(g.check_tableau(c, cur.index)),
                    (Waste | Tableau, Foundation, Some(c)) => {
                        err(check_foundation(&g.foundations[cur.index], c))
                    }
                    _ => MoveError::NotAllowed,
                }
            }
            GameMode::FreeCell => {
                let g = &self.freecell.game;
                let top = match sel.area {
                    FreeCell => g.cell_card(sel.index),
                    Foundation => g.foundation_top(sel.index),
                    Tableau => g.tableau_top(sel.index),
                    _ => None,
                };
                match (sel.area, cur.area, top) {
                    (Tableau, Tableau, _) => {
                        err(g.check_tableau_move(sel.index, cur.index, sel.count))
                    }
                    (FreeCell | Foundation, Tableau, Some(c)) => err(g.check_tableau(c, cur.index)),
                    (FreeCell | Tableau, Foundation, Some(c)) => {
                        err(check_foundation(&g.foundations[cur.index], c))
                    }
                    (Tableau, FreeCell, _) if g.cell_card(cur.index).is_some() => {
                        MoveError::CellOccupied
                    }
                    _ => MoveError::NotAllowed,
                }
            }
            GameMode::Spider => match (sel.area, cur.area) {
                (Tableau, Tableau) => err(self
                    .spider
                    .game
                    .check_tableau_move(sel.index, cur.index, sel.count)),
                _ => MoveError::NotAllowed,
            },
        }
    }

    // ---- mouse ----

    /// A mouse press at screen cell (`x`, `y`). `right` sends the clicked
    /// pile's top card to a foundation.
    pub fn click(&mut self, x: u16, y: u16, right: bool) {
        let pos = Position::new(x, y);
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|r| r.rect.contains(pos))
            .map(|r| r.hit);
        if self.confirm.is_some() {
            self.cancel_pending();
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        match hit {
            None => {
                if self.selected.is_some() {
                    self.cancel_selection();
                }
            }
            Some(Hit::Action(action)) => self.perform(action),
            Some(Hit::MenuGame(mode)) => {
                if self.menu.game == mode {
                    self.menu_confirm();
                } else {
                    self.menu.game = mode;
                }
            }
            Some(Hit::MenuDifficulty(d)) => self.menu.difficulty = d,
            Some(Hit::MenuDraw) => self.menu_toggle_draw(),
            Some(Hit::MenuStart) => self.menu_confirm(),
            Some(Hit::Pile { area, index, card }) => self.click_pile(area, index, card, right),
        }
    }

    fn click_pile(&mut self, area: CursorArea, index: usize, card: Option<usize>, right: bool) {
        if self.screen != Screen::Game || self.is_won() {
            return;
        }
        self.cursor = Cursor { area, index };
        if right {
            self.send_to_foundation();
            return;
        }
        if area == CursorArea::Stock {
            self.draw();
            return;
        }
        match self.selected {
            None => self.grab(card),
            Some(sel) if sel.area == area && sel.index == index => {
                // Clicking a different card of the held pile re-grabs from
                // there; clicking the same spot again lets go.
                let regrab = card
                    .map(|c| self.tableau_len(index).saturating_sub(c))
                    .is_some_and(|n| n != sel.count && (1..=sel.max).contains(&n));
                if regrab {
                    self.grab(card);
                } else {
                    self.cancel_selection();
                }
            }
            Some(sel) => self.place(sel),
        }
    }

    // ---- legal-target highlighting ----

    /// Piles where the current selection could legally go, using the same
    /// counts that placing would try.
    pub fn legal_targets(&self) -> Vec<(CursorArea, usize)> {
        use CursorArea::{Foundation, FreeCell, Tableau, Waste};
        let Some(sel) = self.selected else {
            return Vec::new();
        };
        let counts = sel.counts();
        let mut out = Vec::new();
        match self.mode {
            GameMode::Klondike => {
                let g = &self.klondike.game;
                let single = match sel.area {
                    Waste => g.waste_top(),
                    Foundation => g.foundation_top(sel.index),
                    _ => None,
                };
                for to in 0..crate::klondike::TABLEAU_PILES {
                    let fits = match sel.area {
                        Tableau => counts
                            .iter()
                            .any(|&n| g.check_tableau_move(sel.index, to, n).is_ok()),
                        _ => single.is_some_and(|c| g.can_place_on_tableau(c, to)),
                    };
                    if fits {
                        out.push((Tableau, to));
                    }
                }
                let up = match sel.area {
                    Waste => g.waste_top(),
                    Tableau => g.tableau_top(sel.index),
                    _ => None,
                };
                if let Some(f) = up.and_then(|c| g.foundation_for(c)) {
                    out.push((Foundation, f));
                }
            }
            GameMode::FreeCell => {
                let g = &self.freecell.game;
                let single = match sel.area {
                    FreeCell => g.cell_card(sel.index),
                    Foundation => g.foundation_top(sel.index),
                    _ => None,
                };
                for to in 0..crate::freecell::TABLEAU_PILES {
                    let fits = match sel.area {
                        Tableau => counts
                            .iter()
                            .any(|&n| g.check_tableau_move(sel.index, to, n).is_ok()),
                        _ => single.is_some_and(|c| g.can_place_on_tableau(c, to)),
                    };
                    if fits {
                        out.push((Tableau, to));
                    }
                }
                let up = match sel.area {
                    FreeCell => g.cell_card(sel.index),
                    Tableau => g.tableau_top(sel.index),
                    _ => None,
                };
                if let Some(f) = up.and_then(|c| g.foundation_for(c)) {
                    out.push((Foundation, f));
                }
                if sel.area == Tableau {
                    out.extend(
                        (0..g.num_cells)
                            .filter(|&c| g.cell_card(c).is_none())
                            .map(|c| (FreeCell, c)),
                    );
                }
            }
            GameMode::Spider => {
                let g = &self.spider.game;
                if sel.area == Tableau {
                    out.extend(
                        (0..crate::spider::TABLEAU_PILES)
                            .filter(|&to| {
                                counts
                                    .iter()
                                    .any(|&n| g.check_tableau_move(sel.index, to, n).is_ok())
                            })
                            .map(|to| (Tableau, to)),
                    );
                }
            }
        }
        out
    }
}

/// Klondike deal for the given settings: Easy deals are verified solvable.
fn make_klondike(draw_mode: DrawMode, difficulty: Difficulty, deal: Option<u32>) -> KlondikeGame {
    match deal {
        Some(deal) => KlondikeGame::new(draw_mode, difficulty, deal),
        None if difficulty == Difficulty::Easy => {
            KlondikeGame::new_solvable(draw_mode, difficulty, random_deal())
        }
        None => KlondikeGame::new(draw_mode, difficulty, random_deal()),
    }
}

/// Map position `i` in a row of `from` slots to the nearest of `to` slots.
fn scale_index(i: usize, from: usize, to: usize) -> usize {
    if from <= 1 || to <= 1 {
        return 0;
    }
    (i * (to - 1) + (from - 1) / 2) / (from - 1)
}

fn place_klondike(g: &mut KlondikeGame, sel: Selected, cur: Cursor, counts: &[usize]) -> bool {
    use CursorArea::{Foundation, Tableau, Waste};
    match (sel.area, cur.area) {
        (Waste, Foundation) => g.move_waste_to_foundation(),
        (Tableau, Foundation) => g.move_tableau_to_foundation(sel.index),
        (Waste, Tableau) => g.move_waste_to_tableau(cur.index),
        (Foundation, Tableau) => g.move_foundation_to_tableau(sel.index, cur.index),
        (Tableau, Tableau) => counts
            .iter()
            .any(|&n| g.move_tableau_to_tableau(sel.index, cur.index, n)),
        _ => false,
    }
}

fn place_freecell(g: &mut FreeCellGame, sel: Selected, cur: Cursor, counts: &[usize]) -> bool {
    use CursorArea::{Foundation, FreeCell, Tableau};
    match (sel.area, cur.area) {
        (Tableau, Tableau) => counts
            .iter()
            .any(|&n| g.move_tableau_to_tableau(sel.index, cur.index, n)),
        (FreeCell, Tableau) => g.move_cell_to_tableau(sel.index, cur.index),
        (Foundation, Tableau) => g.move_foundation_to_tableau(sel.index, cur.index),
        (Tableau, Foundation) => g.move_tableau_to_foundation(sel.index),
        (FreeCell, Foundation) => g.move_cell_to_foundation(sel.index),
        (Tableau, FreeCell) => g.move_tableau_top_to_cell(sel.index, cur.index),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::Suit;

    fn table() -> App {
        let mut app = App::new(Difficulty::Normal, None, Some(1));
        app.screen = Screen::Game;
        app
    }

    fn at(area: CursorArea, index: usize) -> Cursor {
        Cursor { area, index }
    }

    fn clear_klondike(app: &mut App) {
        let g = &mut app.klondike.game;
        g.tableau.iter_mut().for_each(Vec::clear);
        g.stock.clear();
        g.waste.clear();
    }

    fn clear_freecell(app: &mut App) {
        let g = &mut app.freecell.game;
        g.tableau.iter_mut().for_each(Vec::clear);
        g.foundations.iter_mut().for_each(Vec::clear);
        g.freecells = [None; crate::freecell::MAX_CELLS];
    }

    #[test]
    fn cursor_stays_in_bounds_per_mode() {
        let mut app = table();
        app.mode = GameMode::Spider;
        app.focus(CursorArea::Tableau, 9);
        assert_eq!(app.cursor.index, 9);
        app.focus(CursorArea::Tableau, 10); // out of range, ignored
        assert_eq!(app.cursor.index, 9);
        app.mode = GameMode::Klondike;
        app.ensure_cursor_valid();
        assert!(app.cursor.index < 7);
    }

    #[test]
    fn arrows_move_spatially_between_rows() {
        let mut app = table();
        app.cursor = at(CursorArea::Tableau, 6);
        app.move_cursor(Dir::Up);
        assert_eq!(
            app.cursor,
            at(CursorArea::Foundation, 3),
            "rightmost over rightmost"
        );
        app.move_cursor(Dir::Left);
        assert_eq!(app.cursor, at(CursorArea::Foundation, 2));
        app.cursor = at(CursorArea::Foundation, 0);
        app.move_cursor(Dir::Left);
        assert_eq!(app.cursor, at(CursorArea::Waste, 0), "left crosses areas");
        app.move_cursor(Dir::Down);
        assert_eq!(app.cursor, at(CursorArea::Tableau, 1));
        app.move_cursor(Dir::Down);
        assert_eq!(
            app.cursor,
            at(CursorArea::Tableau, 1),
            "down stops at the tableau"
        );
    }

    #[test]
    fn undo_restores_freecell_move() {
        let mut app = table();
        app.mode = GameMode::FreeCell;
        clear_freecell(&mut app);
        app.freecell.game.tableau[0].push(Card::up(Suit::Spades, 1));
        app.freecell.game.tableau[1].push(Card::up(Suit::Hearts, 9));
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        app.cursor = at(CursorArea::Foundation, 0);
        app.select_or_place();
        assert_eq!(app.freecell.game.foundations[0].len(), 1);
        app.undo();
        assert_eq!(app.freecell.game.foundations[0].len(), 0);
        assert_eq!(app.freecell.game.tableau[0].len(), 1);
        app.undo();
        assert_eq!(app.status_message, "Nothing to undo.");
    }

    #[test]
    fn freecell_auto_plays_safe_cards_after_a_move() {
        let mut app = table();
        app.mode = GameMode::FreeCell;
        clear_freecell(&mut app);
        let g = &mut app.freecell.game;
        g.tableau[0].extend([Card::up(Suit::Spades, 1), Card::up(Suit::Hearts, 9)]);
        g.tableau[1].push(Card::up(Suit::Clubs, 10));
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        app.cursor = at(CursorArea::Tableau, 1);
        app.select_or_place();
        assert_eq!(app.freecell.game.foundations[0].len(), 1, "A♠ went up");
        app.undo();
        assert_eq!(app.freecell.game.tableau[0].len(), 2, "one undo step");
    }

    #[test]
    fn klondike_grab_and_drop_four_clubs_on_five_diamonds() {
        let mut app = table();
        clear_klondike(&mut app);
        app.klondike.game.tableau[0].push(Card::up(Suit::Diamonds, 5));
        app.klondike.game.tableau[1].push(Card::up(Suit::Clubs, 4));
        app.cursor = at(CursorArea::Tableau, 1);
        app.select_or_place();
        assert!(app.selected.is_some(), "4♣ should be grabbable");
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        assert_eq!(app.klondike.game.tableau[0].len(), 2);
        assert!(app.selected.is_none());
        assert!(
            !app.status_message.contains("Illegal"),
            "{}",
            app.status_message
        );
    }

    #[test]
    fn klondike_same_color_drop_explains_why() {
        let mut app = table();
        clear_klondike(&mut app);
        app.klondike.game.tableau[0].push(Card::up(Suit::Clubs, 5));
        app.klondike.game.tableau[1].push(Card::up(Suit::Spades, 4));
        app.cursor = at(CursorArea::Tableau, 1);
        app.select_or_place();
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        assert_eq!(app.klondike.game.tableau[0].len(), 1);
        assert_eq!(app.status_message, MoveError::SameColor.to_string());
        assert!(app.selected.is_some(), "grab kept for another try");
    }

    #[test]
    fn targets_include_piles_that_take_part_of_the_run() {
        let mut app = table();
        clear_klondike(&mut app);
        let g = &mut app.klondike.game;
        g.tableau[0].extend([
            Card::up(Suit::Clubs, 9),
            Card::up(Suit::Hearts, 8),
            Card::up(Suit::Clubs, 7),
        ]);
        g.tableau[1].push(Card::up(Suit::Spades, 9)); // takes 8♥ 7♣
        g.tableau[2].push(Card::up(Suit::Hearts, 10)); // takes all three
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        let targets = app.legal_targets();
        assert!(targets.contains(&(CursorArea::Tableau, 1)));
        assert!(targets.contains(&(CursorArea::Tableau, 2)));
        // Holding exactly three: only pile 3 fits.
        app.selected.as_mut().unwrap().explicit = true;
        assert_eq!(app.legal_targets(), vec![(CursorArea::Tableau, 2)]);
    }

    #[test]
    fn freecell_can_move_a_single_card_into_an_empty_column() {
        let mut app = table();
        app.mode = GameMode::FreeCell;
        clear_freecell(&mut app);
        let g = &mut app.freecell.game;
        g.tableau[0].extend([Card::up(Suit::Spades, 9), Card::up(Suit::Hearts, 8)]);
        for p in &mut g.tableau[2..] {
            p.push(Card::up(Suit::Clubs, 13));
        }
        app.cursor = at(CursorArea::Tableau, 0);
        app.select_or_place();
        assert_eq!(app.selected.unwrap().count, 2);
        app.adjust_grab(false);
        assert_eq!(app.selected.unwrap().count, 1);
        app.cursor = at(CursorArea::Tableau, 1);
        app.select_or_place();
        assert_eq!(app.freecell.game.tableau[1].len(), 1, "only 8♥ moved");
        assert_eq!(app.freecell.game.tableau[0].len(), 1);
    }

    #[test]
    fn space_on_the_stock_draws() {
        let mut app = table();
        let before = app.klondike.game.stock.len();
        app.cursor = at(CursorArea::Stock, 0);
        app.select_or_place();
        assert!(app.klondike.game.stock.len() < before);
    }

    #[test]
    fn hard_mode_extra_redeal_is_refused_not_game_over() {
        let mut app = App::new(Difficulty::Hard, None, Some(2));
        app.screen = Screen::Game;
        for _ in 0..200 {
            app.draw();
        }
        assert!(!app.klondike.game.can_redeal());
        let depth = app.undo_depth();
        app.draw();
        assert_eq!(app.status_message, "No redeals left on Hard.");
        assert_eq!(app.undo_depth(), depth, "a refused draw adds no undo step");
    }

    #[test]
    fn menu_confirm_applies_spider_difficulty() {
        let mut app = table();
        app.open_menu();
        app.menu.game = GameMode::Spider;
        app.menu.difficulty = Difficulty::Easy;
        app.menu_confirm();
        assert_eq!(app.mode, GameMode::Spider);
        assert_eq!(app.difficulty(), Difficulty::Easy);
        assert_eq!(app.spider.game.suits, SpiderSuits::One);
    }

    #[test]
    fn difficulty_is_tracked_per_game() {
        let mut app = table();
        app.open_menu();
        app.menu.game = GameMode::Klondike;
        app.menu.difficulty = Difficulty::Hard;
        app.menu_confirm();
        app.open_menu();
        app.menu.game = GameMode::FreeCell;
        app.menu.difficulty = Difficulty::Hard;
        app.menu_confirm();
        assert_eq!(app.difficulty(), Difficulty::Hard);
        assert_eq!(app.freecell.game.num_cells, 3, "FreeCell really is on Hard");
    }

    #[test]
    fn menu_resumes_or_asks_before_discarding() {
        let mut app = table();
        app.draw(); // Klondike now in progress
        let stock = app.klondike.game.stock.clone();
        app.open_menu();
        assert!(app.menu_resumes());
        app.menu_confirm();
        assert_eq!(app.screen, Screen::Game);
        assert_eq!(app.klondike.game.stock, stock, "resumed, not re-dealt");

        app.open_menu();
        app.menu.difficulty = Difficulty::Hard;
        assert!(!app.menu_resumes());
        app.menu_confirm();
        assert!(matches!(app.confirm, Some(PendingConfirm::Deal(_))));
        app.cancel_pending();
        assert_eq!(
            app.klondike.difficulty,
            Difficulty::Normal,
            "nothing changed"
        );
        app.menu_confirm();
        app.confirm_pending();
        assert_eq!(app.klondike.difficulty, Difficulty::Hard);
        assert_eq!(app.klondike.game.move_count(), 0);
    }

    #[test]
    fn menu_draw_follows_difficulty_unless_overridden() {
        let mut app = table();
        app.menu.difficulty = Difficulty::Easy;
        assert_eq!(app.menu_draw_mode(), DrawMode::Draw1);
        app.menu.difficulty = Difficulty::Normal;
        assert_eq!(app.menu_draw_mode(), DrawMode::Draw3);
        app.menu_toggle_draw();
        assert_eq!(app.menu_draw_mode(), DrawMode::Draw1);
    }

    #[test]
    fn destructive_actions_confirm_only_with_progress() {
        let mut app = table();
        app.perform(Action::Quit);
        assert!(app.should_quit, "nothing to lose: quit at once");

        let mut app = table();
        app.draw();
        app.perform(Action::Quit);
        assert!(!app.should_quit, "quit waits for confirmation");
        app.cancel_pending();
        assert!(app.confirm.is_none());

        let before = app.klondike.game.tableau.clone();
        app.perform(Action::New);
        assert_eq!(app.klondike.game.tableau, before, "pending, untouched");
        app.cancel_pending();
        assert_eq!(app.klondike.game.tableau, before);
    }

    #[test]
    fn restart_replays_the_same_deal() {
        let mut app = table();
        let tableau = app.klondike.game.tableau.clone();
        app.draw();
        app.perform(Action::Restart);
        app.confirm_pending();
        assert_eq!(app.klondike.game.tableau, tableau);
        assert_eq!(app.klondike.game.move_count(), 0);
        assert_eq!(app.undo_depth(), 0);
    }

    #[test]
    fn clock_runs_only_while_playing() {
        let mut app = table();
        app.screen = Screen::Menu;
        app.tick();
        assert!(app.klondike.clock.since.is_none(), "paused on the menu");
        app.screen = Screen::Game;
        app.tick();
        assert!(app.klondike.clock.since.is_some());
        assert!(app.freecell.clock.since.is_none(), "other games paused");
        app.show_help = true;
        app.tick();
        assert!(app.klondike.clock.since.is_none(), "paused under help");
    }

    #[test]
    fn mouse_click_grabs_from_a_card_and_places() {
        let mut app = table();
        clear_klondike(&mut app);
        let g = &mut app.klondike.game;
        g.tableau[0].extend([Card::up(Suit::Clubs, 9), Card::up(Suit::Hearts, 8)]);
        g.tableau[1].push(Card::up(Suit::Spades, 9));
        let pile = |index, card| Hit::Pile {
            area: CursorArea::Tableau,
            index,
            card,
        };
        app.hits = vec![
            HitRegion {
                rect: Rect::new(0, 0, 5, 1),
                hit: pile(0, Some(0)),
            },
            HitRegion {
                rect: Rect::new(0, 1, 5, 1),
                hit: pile(0, Some(1)),
            },
            HitRegion {
                rect: Rect::new(10, 0, 5, 5),
                hit: pile(1, Some(0)),
            },
        ];
        app.click(1, 1, false); // grab 8♥ only
        assert_eq!(app.selected.map(|s| (s.count, s.explicit)), Some((1, true)));
        app.click(11, 2, false); // onto 9♠
        assert_eq!(app.klondike.game.tableau[1].len(), 2);
        app.click(40, 40, false); // empty felt: nothing happens
        assert!(app.selected.is_none());
    }

    #[test]
    fn scale_index_maps_rows() {
        assert_eq!(scale_index(0, 7, 6), 0);
        assert_eq!(scale_index(6, 7, 6), 5);
        assert_eq!(scale_index(3, 10, 1), 0);
        assert_eq!(scale_index(0, 1, 10), 0);
    }
}
