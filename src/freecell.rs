//! Full FreeCell solitaire state and rules.
//!
//! Layout: 8 tableau columns, N free cells, 4 foundations.
//! - Tableau builds down by one with alternating colors; any card (or run)
//!   may move into an empty column.
//! - Foundations build up by suit from the Ace.
//! - Each free cell holds one card.
//! - Moving a run uses supermove capacity:
//!   `max = (1 + free_cells) * 2^(empty_columns excluding destination)`.
//! - Difficulty selects the free-cell count: Easy = 5, Normal = 4, Hard = 3.

use std::fmt;

use crate::cards::{Card, is_valid_descending_alternating_run, shuffled_deck};
use crate::rules::{
    Difficulty, MoveError, Solitaire, cards, check_alternating, foundation_for,
    is_safe_for_foundation,
};

pub const TABLEAU_PILES: usize = 8;
pub const FOUNDATIONS: usize = 4;
/// Most free cells any difficulty uses (Easy).
pub const MAX_CELLS: usize = 5;

/// Score: only foundation progress counts, so shuffling cards between
/// cells and columns can't farm points.
pub const SCORE_TO_FOUNDATION: i32 = 10;
pub const SCORE_FOUNDATION_TO_TABLEAU: i32 = -10;
pub const SCORE_WIN_BONUS: i32 = 500;

/// Free-cell count per difficulty: Easy = 5, Normal = 4, Hard = 3.
pub fn cell_count_for(difficulty: Difficulty) -> usize {
    match difficulty {
        Difficulty::Easy => 5,
        Difficulty::Normal => 4,
        Difficulty::Hard => 3,
    }
}

/// Full FreeCell game state.
#[derive(Debug, Clone)]
pub struct FreeCellGame {
    /// Free-cell slots; only `freecells[..num_cells]` are in play.
    pub freecells: [Option<Card>; MAX_CELLS],
    pub num_cells: usize,
    pub foundations: [Vec<Card>; FOUNDATIONS],
    pub tableau: [Vec<Card>; TABLEAU_PILES],
    score: i32,
    moves: u32,
    deal: u32,
}

impl FreeCellGame {
    pub fn new(difficulty: Difficulty, deal: u32) -> Self {
        let mut game = Self {
            freecells: [None; MAX_CELLS],
            num_cells: cell_count_for(difficulty),
            foundations: Default::default(),
            tableau: Default::default(),
            score: 0,
            moves: 0,
            deal,
        };
        game.restart();
        game
    }

    /// Re-deal the same deal number: all 52 cards face up across 8 columns.
    pub fn restart(&mut self) {
        for pile in &mut self.tableau {
            pile.clear();
        }
        for f in &mut self.foundations {
            f.clear();
        }
        self.freecells = [None; MAX_CELLS];
        for (i, mut c) in shuffled_deck(self.deal).into_iter().enumerate() {
            c.face_up = true;
            self.tableau[i % TABLEAU_PILES].push(c);
        }
        self.score = 0;
        self.moves = 0;
    }

    // ---- accessors ----

    pub fn active_cells(&self) -> &[Option<Card>] {
        &self.freecells[..self.num_cells]
    }

    /// Number of empty free cells.
    pub fn free_count(&self) -> usize {
        self.active_cells().iter().filter(|c| c.is_none()).count()
    }

    pub fn tableau_top(&self, pile: usize) -> Option<Card> {
        self.tableau.get(pile)?.last().copied()
    }

    pub fn cell_card(&self, cell: usize) -> Option<Card> {
        self.active_cells().get(cell).copied().flatten()
    }

    pub fn foundation_top(&self, pile: usize) -> Option<Card> {
        self.foundations.get(pile)?.last().copied()
    }

    pub fn foundation_for(&self, card: Card) -> Option<usize> {
        foundation_for(&self.foundations, card)
    }

    // ---- rules ----

    /// Tableau rule: empty takes anything; otherwise alternating colors,
    /// exactly one rank lower.
    pub fn check_tableau(&self, card: Card, tableau: usize) -> Result<(), MoveError> {
        let pile = self.tableau.get(tableau).ok_or(MoveError::NotAllowed)?;
        match pile.last() {
            None => Ok(()),
            Some(&top) => check_alternating(top, card),
        }
    }

    pub fn can_place_on_tableau(&self, card: Card, tableau: usize) -> bool {
        self.check_tableau(card, tableau).is_ok()
    }

    /// Largest movable suffix of a tableau pile (the whole valid run).
    pub fn grab_count_tableau(&self, pile: usize) -> usize {
        let Some(p) = self.tableau.get(pile) else {
            return 0;
        };
        (0..p.len())
            .find(|&start| is_valid_descending_alternating_run(&p[start..]))
            .map_or(0, |start| p.len() - start)
    }

    fn empty_columns_excluding(&self, dest: usize) -> usize {
        self.tableau
            .iter()
            .enumerate()
            .filter(|&(i, p)| i != dest && p.is_empty())
            .count()
    }

    /// Supermove capacity for `dest`: `(1 + free_cells) * 2^empty_others`.
    pub fn max_movable(&self, dest: usize) -> usize {
        (self.free_count() + 1) << self.empty_columns_excluding(dest)
    }

    /// Whether the top `count` cards of `from` may move onto `to`.
    pub fn check_tableau_move(
        &self,
        from: usize,
        to: usize,
        count: usize,
    ) -> Result<(), MoveError> {
        if from >= TABLEAU_PILES || to >= TABLEAU_PILES || from == to {
            return Err(MoveError::NotAllowed);
        }
        let pile = &self.tableau[from];
        if count == 0 || count > pile.len() {
            return Err(MoveError::NotARun);
        }
        let start = pile.len() - count;
        if !is_valid_descending_alternating_run(&pile[start..]) {
            return Err(MoveError::NotARun);
        }
        let max = self.max_movable(to);
        if count > max {
            return Err(MoveError::TooMany { count, max });
        }
        self.check_tableau(pile[start], to)
    }

    fn add_score(&mut self, delta: i32) {
        self.score = (self.score + delta).max(0);
    }

    fn on_foundation_move(&mut self) {
        self.add_score(SCORE_TO_FOUNDATION);
        self.moves += 1;
        if self.is_won() {
            self.add_score(SCORE_WIN_BONUS);
        }
    }

    // ---- moves ----

    /// Move exactly the top `count` cards of one tableau pile to another.
    pub fn move_tableau_to_tableau(&mut self, from: usize, to: usize, count: usize) -> bool {
        if self.check_tableau_move(from, to, count).is_err() {
            return false;
        }
        let start = self.tableau[from].len() - count;
        let moving: Vec<Card> = self.tableau[from].drain(start..).collect();
        self.tableau[to].extend(moving);
        self.moves += 1;
        true
    }

    /// Tableau top card -> the foundation that accepts it.
    pub fn move_tableau_to_foundation(&mut self, from: usize) -> bool {
        let Some(card) = self.tableau_top(from) else {
            return false;
        };
        let Some(f) = self.foundation_for(card) else {
            return false;
        };
        self.tableau[from].pop();
        self.foundations[f].push(card);
        self.on_foundation_move();
        true
    }

    /// Tableau top card -> a specific empty free cell.
    pub fn move_tableau_top_to_cell(&mut self, from: usize, cell: usize) -> bool {
        if cell >= self.num_cells || self.freecells[cell].is_some() {
            return false;
        }
        let Some(card) = self.tableau.get_mut(from).and_then(Vec::pop) else {
            return false;
        };
        self.freecells[cell] = Some(card);
        self.moves += 1;
        true
    }

    /// First empty free cell, if any.
    #[cfg(test)]
    pub fn first_empty_cell(&self) -> Option<usize> {
        self.active_cells().iter().position(Option::is_none)
    }

    /// Free cell -> tableau pile.
    pub fn move_cell_to_tableau(&mut self, cell: usize, to: usize) -> bool {
        let Some(card) = self.cell_card(cell) else {
            return false;
        };
        if !self.can_place_on_tableau(card, to) {
            return false;
        }
        self.freecells[cell] = None;
        self.tableau[to].push(card);
        self.moves += 1;
        true
    }

    /// Free cell -> the foundation that accepts it.
    pub fn move_cell_to_foundation(&mut self, cell: usize) -> bool {
        let Some(card) = self.cell_card(cell) else {
            return false;
        };
        let Some(f) = self.foundation_for(card) else {
            return false;
        };
        self.freecells[cell] = None;
        self.foundations[f].push(card);
        self.on_foundation_move();
        true
    }

    /// Foundation top back to a tableau pile (costs points).
    pub fn move_foundation_to_tableau(&mut self, foundation: usize, to: usize) -> bool {
        let Some(card) = self.foundation_top(foundation) else {
            return false;
        };
        if !self.can_place_on_tableau(card, to) {
            return false;
        }
        self.foundations[foundation].pop();
        self.tableau[to].push(card);
        self.add_score(SCORE_FOUNDATION_TO_TABLEAU);
        self.moves += 1;
        true
    }

    fn is_safe(&self, card: Card) -> bool {
        is_safe_for_foundation(&self.foundations, card) && self.foundation_for(card).is_some()
    }

    /// Move every currently-safe card (tableau tops + cells) to the
    /// foundations. Returns the number of cards moved.
    pub fn auto_move_safe(&mut self) -> usize {
        let mut moved = 0;
        loop {
            if let Some(t) =
                (0..TABLEAU_PILES).find(|&t| self.tableau_top(t).is_some_and(|c| self.is_safe(c)))
            {
                self.move_tableau_to_foundation(t);
            } else if let Some(c) =
                (0..self.num_cells).find(|&c| self.cell_card(c).is_some_and(|c| self.is_safe(c)))
            {
                self.move_cell_to_foundation(c);
            } else {
                return moved;
            }
            moved += 1;
        }
    }

    /// Every column is already in descending order (each card no higher
    /// than the one beneath it), so the rest can go up without any more
    /// decisions.
    pub fn can_auto_finish(&self) -> bool {
        !self.is_won()
            && self
                .tableau
                .iter()
                .all(|p| p.windows(2).all(|w| w[1].rank <= w[0].rank))
    }

    /// Move everything to the foundations (only when [`can_auto_finish`]).
    ///
    /// [`can_auto_finish`]: FreeCellGame::can_auto_finish
    pub fn auto_finish(&mut self) -> usize {
        if !self.can_auto_finish() {
            return 0;
        }
        let mut moved = 0;
        loop {
            if let Some(t) = (0..TABLEAU_PILES).find(|&t| {
                self.tableau_top(t)
                    .is_some_and(|c| self.foundation_for(c).is_some())
            }) {
                self.move_tableau_to_foundation(t);
            } else if let Some(c) = (0..self.num_cells).find(|&c| {
                self.cell_card(c)
                    .is_some_and(|c| self.foundation_for(c).is_some())
            }) {
                self.move_cell_to_foundation(c);
            } else {
                return moved;
            }
            moved += 1;
        }
    }

    /// First foundation move from a tableau top or cell.
    fn foundation_move(&self, safe_only: bool) -> Option<Hint> {
        let ok = |c: Card| {
            self.foundation_for(c).is_some()
                && (!safe_only || is_safe_for_foundation(&self.foundations, c))
        };
        (0..TABLEAU_PILES)
            .find(|&t| self.tableau_top(t).is_some_and(ok))
            .map(Hint::TableauToFoundation)
            .or_else(|| {
                (0..self.num_cells)
                    .find(|&c| self.cell_card(c).is_some_and(ok))
                    .map(Hint::CellToFoundation)
            })
    }

    /// Next useful move. Only progress moves are suggested: each sends a
    /// card up, frees a cell, strictly increases the number of built pairs,
    /// or uncovers a card that goes up next, so following hints never loops.
    pub fn best_move(&self) -> Option<Hint> {
        if self.is_won() {
            return None;
        }
        if self.can_auto_finish() {
            return Some(Hint::AutoFinish);
        }
        // Safe foundation moves first, then any other foundation move.
        if let Some(h) = self
            .foundation_move(true)
            .or_else(|| self.foundation_move(false))
        {
            return Some(h);
        }
        // Free a cell onto a building column.
        for cell in 0..self.num_cells {
            if let Some(card) = self.cell_card(cell)
                && let Some(to) = (0..TABLEAU_PILES)
                    .find(|&t| !self.tableau[t].is_empty() && self.can_place_on_tableau(card, t))
            {
                return Some(Hint::CellToTableau { cell, to });
            }
        }
        // Build: move a whole run onto a card it continues.
        for from in 0..TABLEAU_PILES {
            let count = self.grab_count_tableau(from);
            if count == 0 {
                continue;
            }
            if let Some(to) = (0..TABLEAU_PILES).find(|&to| {
                !self.tableau[to].is_empty() && self.check_tableau_move(from, to, count).is_ok()
            }) {
                return Some(Hint::TableauToTableau { from, to, count });
            }
        }
        // Uncover a card that can go straight to a foundation.
        for from in 0..TABLEAU_PILES {
            let len = self.tableau[from].len();
            for count in 1..len {
                if self
                    .foundation_for(self.tableau[from][len - count - 1])
                    .is_none()
                {
                    continue;
                }
                if let Some(to) =
                    (0..TABLEAU_PILES).find(|&to| self.check_tableau_move(from, to, count).is_ok())
                {
                    return Some(Hint::TableauToTableau { from, to, count });
                }
                if count == 1 && self.free_count() > 0 {
                    return Some(Hint::TableauToCell(from));
                }
            }
        }
        None
    }

    /// Carry out a suggested move.
    #[cfg(test)]
    pub fn apply(&mut self, hint: Hint) -> bool {
        match hint {
            Hint::TableauToFoundation(t) => self.move_tableau_to_foundation(t),
            Hint::CellToFoundation(c) => self.move_cell_to_foundation(c),
            Hint::CellToTableau { cell, to } => self.move_cell_to_tableau(cell, to),
            Hint::TableauToTableau { from, to, count } => {
                self.move_tableau_to_tableau(from, to, count)
            }
            Hint::TableauToCell(from) => self
                .first_empty_cell()
                .is_some_and(|cell| self.move_tableau_top_to_cell(from, cell)),
            Hint::AutoFinish => self.auto_finish() > 0,
        }
    }
}

/// A move suggested by [`FreeCellGame::best_move`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    TableauToFoundation(usize),
    CellToFoundation(usize),
    CellToTableau {
        cell: usize,
        to: usize,
    },
    TableauToTableau {
        from: usize,
        to: usize,
        count: usize,
    },
    TableauToCell(usize),
    AutoFinish,
}

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Hint::TableauToFoundation(t) => {
                write!(f, "Move the top card of tableau {} to a foundation.", t + 1)
            }
            Hint::CellToFoundation(c) => {
                write!(f, "Move the card in cell {} to a foundation.", c + 1)
            }
            Hint::CellToTableau { cell, to } => {
                write!(
                    f,
                    "Move the card in cell {} to tableau {}.",
                    cell + 1,
                    to + 1
                )
            }
            Hint::TableauToTableau { from, to, count } => write!(
                f,
                "Move {} from tableau {} to tableau {}.",
                cards(count),
                from + 1,
                to + 1
            ),
            Hint::TableauToCell(t) => write!(
                f,
                "Park the top card of tableau {} in a free cell to free a foundation card.",
                t + 1
            ),
            Hint::AutoFinish => f.write_str("Every column is in order: press a to finish."),
        }
    }
}

impl Solitaire for FreeCellGame {
    fn score(&self) -> i32 {
        self.score
    }

    fn move_count(&self) -> u32 {
        self.moves
    }

    fn is_won(&self) -> bool {
        self.foundations.iter().map(Vec::len).sum::<usize>() == 52
    }

    fn deal(&self) -> u32 {
        self.deal
    }

    fn hint(&self) -> Option<String> {
        if let Some(h) = self.best_move() {
            return Some(h.to_string());
        }
        let empty_columns = self.tableau.iter().filter(|p| p.is_empty()).count();
        (self.free_count() > 0 || empty_columns > 0).then(|| {
            "No direct progress: park cards in free cells or empty columns to dig out low cards."
                .to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::Suit;

    fn card(suit: Suit, v: u8) -> Card {
        Card::up(suit, v)
    }

    fn empty_game() -> FreeCellGame {
        let mut g = FreeCellGame::new(Difficulty::Normal, 1);
        g.tableau.iter_mut().for_each(Vec::clear);
        g
    }

    #[test]
    fn deal_spreads_52_face_up_and_is_reproducible() {
        let mut g = FreeCellGame::new(Difficulty::Normal, 11);
        assert_eq!(g.tableau.iter().map(Vec::len).sum::<usize>(), 52);
        assert!(g.tableau.iter().flatten().all(|c| c.face_up));
        assert_eq!(g.num_cells, 4);
        let first = g.tableau.clone();
        g.move_tableau_top_to_cell(0, 0);
        g.restart();
        assert_eq!(g.tableau, first);
        assert!(g.freecells.iter().all(Option::is_none));
    }

    #[test]
    fn difficulty_selects_cell_count() {
        assert_eq!(cell_count_for(Difficulty::Easy), 5);
        assert_eq!(cell_count_for(Difficulty::Normal), 4);
        assert_eq!(cell_count_for(Difficulty::Hard), 3);
    }

    #[test]
    fn tableau_rule_alternating_down() {
        let mut g = empty_game();
        g.tableau[0].push(card(Suit::Spades, 7));
        assert!(g.can_place_on_tableau(card(Suit::Hearts, 6), 0));
        assert_eq!(
            g.check_tableau(card(Suit::Clubs, 6), 0),
            Err(MoveError::SameColor)
        );
        assert_eq!(
            g.check_tableau(card(Suit::Hearts, 5), 0),
            Err(MoveError::WrongRank)
        );
        assert!(g.can_place_on_tableau(card(Suit::Clubs, 13), 1)); // empty takes any
    }

    #[test]
    fn supermove_capacity_scales() {
        let mut g = empty_game();
        for p in &mut g.tableau {
            p.push(card(Suit::Clubs, 13));
        }
        assert_eq!(g.max_movable(0), 5);
        g.tableau[2].clear();
        assert_eq!(g.max_movable(0), 10);
        assert_eq!(g.max_movable(2), 5, "destination itself doesn't count");
    }

    #[test]
    fn exact_count_moves_and_capacity_errors() {
        let mut g = empty_game();
        for p in &mut g.tableau[3..] {
            p.push(card(Suit::Clubs, 13));
        }
        g.freecells = [Some(card(Suit::Hearts, 13)); MAX_CELLS];
        g.tableau[0].extend([
            card(Suit::Spades, 9),
            card(Suit::Hearts, 8),
            card(Suit::Clubs, 7),
        ]);
        // No free cells, two other empty columns: capacity 4 into column 1,
        // so a single card can be moved alone.
        assert!(g.move_tableau_to_tableau(0, 1, 1));
        assert_eq!(g.tableau[1].len(), 1);
        assert_eq!(g.tableau[0].len(), 2);

        // Fill every column and cell: capacity drops to 1.
        g.tableau[2].push(card(Suit::Diamonds, 13));
        g.tableau[5].push(card(Suit::Diamonds, 10));
        assert_eq!(
            g.check_tableau_move(0, 5, 2),
            Err(MoveError::TooMany { count: 2, max: 1 })
        );
    }

    #[test]
    fn black_four_onto_red_five_is_legal() {
        let mut g = empty_game();
        g.tableau[0].push(card(Suit::Diamonds, 5));
        g.tableau[1].push(card(Suit::Clubs, 4));
        assert!(g.move_tableau_to_tableau(1, 0, 1));
        assert_eq!(g.tableau[0].len(), 2);
    }

    #[test]
    fn safe_auto_move_respects_unstarted_suits() {
        let mut g = empty_game();
        g.foundations[0] = vec![card(Suit::Spades, 1), card(Suit::Spades, 2)];
        g.foundations[1] = vec![card(Suit::Hearts, 1), card(Suit::Hearts, 2)];
        g.tableau[0].push(card(Suit::Spades, 3));
        g.tableau[1].extend([card(Suit::Diamonds, 2), card(Suit::Clubs, 9)]);
        assert_eq!(g.auto_move_safe(), 0, "3♠ may still be needed for 2♦");
        g.tableau[2].push(card(Suit::Diamonds, 1));
        assert_eq!(g.auto_move_safe(), 1, "A♦ goes up");
    }

    #[test]
    fn auto_finish_only_for_ordered_columns() {
        let mut g = empty_game();
        g.tableau[0].extend([card(Suit::Spades, 2), card(Suit::Hearts, 1)]);
        g.tableau[1].push(card(Suit::Spades, 1));
        assert!(g.can_auto_finish());
        assert_eq!(g.auto_finish(), 3);

        let mut g = empty_game();
        g.tableau[0].extend([card(Suit::Spades, 1), card(Suit::Hearts, 2)]);
        assert!(!g.can_auto_finish());
        assert_eq!(g.auto_finish(), 0);
    }

    #[test]
    fn following_hints_terminates() {
        for deal in 0..20 {
            let mut g = FreeCellGame::new(Difficulty::Hard, deal);
            let mut seen = std::collections::HashSet::new();
            while let Some(h) = g.best_move() {
                let key = format!("{:?}{:?}{:?}", g.tableau, g.freecells, g.foundations);
                assert!(
                    seen.insert(key),
                    "deal {deal}: hint revisited a position ({h})"
                );
                assert!(g.apply(h), "deal {deal}: hint not applicable: {h}");
            }
        }
    }
}
