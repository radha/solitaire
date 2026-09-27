//! Full FreeCell solitaire state and rules.
//!
//! Layout: 8 tableau columns, N freecells, 4 foundations.
//! - Tableau builds down by 1 with alternating colors; only Kings (or any
//!   card) may occupy an empty column.
//! - Foundations build up by suit from the Ace.
//! - Each freecell holds at most 1 card.
//! - Moving a sequence uses supermove capacity:
//!   `max = (1 + free_cells) * 2^(empty_columns excluding destination)`.
//! - Difficulty selects the freecell count: Easy = 5, Normal = 4, Hard = 3.

use crate::cards::{is_valid_descending_alternating_run, shuffled_deck, Card};
use crate::klondike::Difficulty;

/// Maximum freecell slots (Easy uses 5, so the array is bigger than 4).
pub const MAX_CELLS: usize = 6;

/// Score awards.
pub const SCORE_TO_FOUNDATION: i32 = 10;
pub const SCORE_CELL_MOVE: i32 = 5;
pub const SCORE_TABLEAU_MOVE: i32 = 3;
pub const SCORE_WIN_BONUS: i32 = 500;

/// Full FreeCell game state.
#[derive(Debug, Clone)]
pub struct FreeCellGame {
    /// Freecell slots; only `freecells[..num_cells]` are in play.
    pub freecells: [Option<Card>; MAX_CELLS],
    /// Active freecell count (set by difficulty).
    pub num_cells: usize,
    pub foundations: [Vec<Card>; 4],
    pub tableau: [Vec<Card>; 8],
    score: i32,
    moves: u32,
    win_bonus_awarded: bool,
}

impl Default for FreeCellGame {
    fn default() -> Self {
        Self::new()
    }
}

impl FreeCellGame {
    pub fn new() -> Self {
        Self::new_with_difficulty(Difficulty::Normal)
    }

    pub fn new_with_difficulty(difficulty: Difficulty) -> Self {
        let mut game = Self {
            freecells: [None, None, None, None, None, None],
            num_cells: cell_count_for(difficulty),
            foundations: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            tableau: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            score: 0,
            moves: 0,
            win_bonus_awarded: false,
        };
        game.reset();
        game
    }

    /// Deal all 52 cards face-up across the 8 tableau columns.
    pub fn reset(&mut self) {
        let mut deck = shuffled_deck();
        for pile in &mut self.tableau {
            pile.clear();
        }
        for f in &mut self.foundations {
            f.clear();
        }
        self.freecells = [None, None, None, None, None, None];
        let mut i = 0;
        while let Some(mut c) = deck.pop() {
            c.face_up = true;
            self.tableau[i % 8].push(c);
            i += 1;
        }
        self.score = 0;
        self.moves = 0;
        self.win_bonus_awarded = false;
    }

    // ---- accessors ----

    pub fn score(&self) -> i32 {
        self.score
    }

    pub fn move_count(&self) -> u32 {
        self.moves
    }

    pub fn active_cells(&self) -> &[Option<Card>] {
        &self.freecells[..self.num_cells]
    }

    pub fn free_count(&self) -> usize {
        self.active_cells().iter().filter(|c| c.is_none()).count()
    }

    pub fn tableau_top(&self, pile: usize) -> Option<Card> {
        self.tableau.get(pile)?.last().copied()
    }

    pub fn cell_card(&self, cell: usize) -> Option<Card> {
        if cell >= self.num_cells {
            return None;
        }
        self.freecells.get(cell).copied().flatten()
    }

    pub fn foundation_top(&self, pile: usize) -> Option<Card> {
        self.foundations.get(pile)?.last().copied()
    }

    pub fn is_won(&self) -> bool {
        self.foundations.iter().map(|f| f.len()).sum::<usize>() == 52
    }

    // ---- rules ----

    /// Foundation rule: empty takes an Ace; otherwise same suit ascending.
    pub fn can_place_on_foundation(&self, card: Card, foundation: usize) -> bool {
        let pile = match self.foundations.get(foundation) {
            Some(p) => p,
            None => return false,
        };
        match pile.last() {
            None => card.rank.is_ace(),
            Some(top) => top.suit == card.suit && card.rank.value() == top.rank.value() + 1,
        }
    }

    /// Tableau rule: empty takes anything; otherwise alternating colors,
    /// exactly one rank lower.
    pub fn can_place_on_tableau(&self, card: Card, tableau: usize) -> bool {
        let pile = match self.tableau.get(tableau) {
            Some(p) => p,
            None => return false,
        };
        match pile.last() {
            None => true,
            Some(top) => top.color() != card.color() && top.rank.value() == card.rank.value() + 1,
        }
    }

    /// A movable stack must be a face-up alternating-descending sequence.
    pub fn is_valid_sequence(cards: &[Card]) -> bool {
        is_valid_descending_alternating_run(cards)
    }

    /// Largest movable suffix length of a tableau pile (whole valid run).
    pub fn grab_count_tableau(&self, pile: usize) -> usize {
        let p = match self.tableau.get(pile) {
            Some(p) => p,
            None => return 0,
        };
        let len = p.len();
        for start in 0..len {
            if Self::is_valid_sequence(&p[start..]) {
                return len - start;
            }
        }
        0
    }

    fn empty_columns_excluding(&self, dest: Option<usize>) -> usize {
        self.tableau
            .iter()
            .enumerate()
            .filter(|(i, p)| Some(*i) != dest && p.is_empty())
            .count()
    }

    /// Supermove capacity for `dest`: `(1 + free_cells) * 2^empty_others`.
    pub fn max_movable(&self, dest: usize) -> usize {
        let empty = self.empty_columns_excluding(Some(dest));
        (self.free_count() + 1) * (1usize << empty.min(10))
    }

    fn on_foundation_move(&mut self) {
        self.score += SCORE_TO_FOUNDATION;
        self.moves += 1;
        if self.is_won() && !self.win_bonus_awarded {
            self.win_bonus_awarded = true;
            self.score += SCORE_WIN_BONUS;
        }
    }

    // ---- moves ----

    /// Move the top `count` cards of one tableau pile to another pile.
    /// Over-long requests shrink automatically to the largest fitting valid
    /// suffix, so UI grabs can pass the whole run.
    pub fn move_tableau_to_tableau(&mut self, from: usize, mut count: usize, to: usize) -> bool {
        if from >= 8 || to >= 8 || from == to || count == 0 {
            return false;
        }
        let len = self.tableau[from].len();
        if len == 0 {
            return false;
        }
        count = count.min(len);
        let capacity = self.max_movable(to);
        while count > 0 {
            let start = len - count;
            if count <= capacity
                && Self::is_valid_sequence(&self.tableau[from][start..])
                && self.can_place_on_tableau(self.tableau[from][start], to)
            {
                let mut moving: Vec<Card> = self.tableau[from].drain(start..).collect();
                self.tableau[to].append(&mut moving);
                self.score += SCORE_TABLEAU_MOVE;
                self.moves += 1;
                return true;
            }
            count -= 1;
        }
        false
    }

    /// Tableau top card -> any legal foundation.
    pub fn move_tableau_to_foundation(&mut self, from: usize) -> bool {
        let card = match self.tableau_top(from) {
            Some(c) => c,
            None => return false,
        };
        let target = (0..4).find(|&f| self.can_place_on_foundation(card, f));
        match target {
            Some(f) => {
                self.tableau[from].pop();
                self.foundations[f].push(card);
                self.on_foundation_move();
                true
            }
            None => false,
        }
    }

    /// Tableau top card -> a specific empty freecell.
    pub fn move_tableau_top_to_cell(&mut self, from: usize, cell: usize) -> bool {
        if from >= 8 || cell >= self.num_cells || self.freecells[cell].is_some() {
            return false;
        }
        let card = match self.tableau_top(from) {
            Some(c) => c,
            None => return false,
        };
        self.tableau[from].pop();
        self.freecells[cell] = Some(card);
        self.score += SCORE_CELL_MOVE;
        self.moves += 1;
        true
    }

    /// Freecell -> tableau pile.
    pub fn move_cell_to_tableau(&mut self, cell: usize, to: usize) -> bool {
        if to >= 8 {
            return false;
        }
        let card = match self.cell_card(cell) {
            Some(c) => c,
            None => return false,
        };
        if !self.can_place_on_tableau(card, to) {
            return false;
        }
        self.freecells[cell] = None;
        self.tableau[to].push(card);
        self.score += SCORE_CELL_MOVE;
        self.moves += 1;
        true
    }

    /// Freecell -> any legal foundation.
    pub fn move_cell_to_foundation(&mut self, cell: usize) -> bool {
        let card = match self.cell_card(cell) {
            Some(c) => c,
            None => return false,
        };
        let target = (0..4).find(|&f| self.can_place_on_foundation(card, f));
        match target {
            Some(f) => {
                self.freecells[cell] = None;
                self.foundations[f].push(card);
                self.on_foundation_move();
                true
            }
            None => false,
        }
    }

    /// Foundation top back to a tableau pile (undo aid, costs points).
    pub fn move_foundation_to_tableau(&mut self, foundation: usize, to: usize) -> bool {
        if foundation >= 4 || to >= 8 {
            return false;
        }
        let card = match self.foundation_top(foundation) {
            Some(c) => c,
            None => return false,
        };
        if !self.can_place_on_tableau(card, to) {
            return false;
        }
        self.foundations[foundation].pop();
        self.tableau[to].push(card);
        self.score = self.score.saturating_sub(SCORE_TO_FOUNDATION).max(0);
        self.moves += 1;
        true
    }

    /// A card is safe to auto-move when it can never be needed for tableau
    /// building: rank <= 2, or both opposite-color foundations already hold
    /// rank - 1 (classic FreeCell auto rule).
    fn is_safe_to_auto(&self, card: Card) -> bool {
        if card.rank.value() <= 2 {
            return true;
        }
        let need = card.rank.value() - 1;
        let opposite_ok = |red: bool| {
            self.foundations
                .iter()
                .filter(|f| f.last().map(|t| t.suit.is_red()) == Some(red))
                .map(|f| f.last().map(|t| t.rank.value()).unwrap_or(0))
                .min()
                .unwrap_or(0)
                >= need
        };
        // Both colors present from the start in a full deck, so check the
        // minimum top rank per color; missing color counts as 0.
        let red_min = self
            .foundations
            .iter()
            .filter_map(|f| f.last())
            .filter(|t| t.suit.is_red())
            .map(|t| t.rank.value())
            .min()
            .unwrap_or(0);
        let black_min = self
            .foundations
            .iter()
            .filter_map(|f| f.last())
            .filter(|t| !t.suit.is_red())
            .map(|t| t.rank.value())
            .min()
            .unwrap_or(0);
        let _ = opposite_ok;
        if card.suit.is_red() {
            black_min >= need
        } else {
            red_min >= need
        }
    }

    /// Move every currently-safe card (tableau tops + cells) to foundations.
    /// Returns the number of cards moved.
    pub fn auto_move_safe_to_foundations(&mut self) -> usize {
        let mut moved = 0;
        loop {
            let mut progress = false;
            for t in 0..8 {
                if let Some(c) = self.tableau_top(t) {
                    if self.is_safe_to_auto(c) && (0..4).any(|f| self.can_place_on_foundation(c, f))
                    {
                        self.move_tableau_to_foundation(t);
                        moved += 1;
                        progress = true;
                    }
                }
            }
            for c in 0..self.num_cells {
                if let Some(card) = self.cell_card(c) {
                    if self.is_safe_to_auto(card)
                        && (0..4).any(|f| self.can_place_on_foundation(card, f))
                    {
                        self.move_cell_to_foundation(c);
                        moved += 1;
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        moved
    }

    /// Move ALL possible cards to foundations (used to finish trivially-won
    /// positions). Returns the number of cards moved.
    pub fn auto_move_all_to_foundations(&mut self) -> usize {
        let mut moved = 0;
        loop {
            let mut progress = false;
            for t in 0..8 {
                if let Some(c) = self.tableau_top(t) {
                    if (0..4).any(|f| self.can_place_on_foundation(c, f)) {
                        self.move_tableau_to_foundation(t);
                        moved += 1;
                        progress = true;
                    }
                }
            }
            for c in 0..self.num_cells {
                if let Some(card) = self.cell_card(c) {
                    if (0..4).any(|f| self.can_place_on_foundation(card, f)) {
                        self.move_cell_to_foundation(c);
                        moved += 1;
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        moved
    }

    /// Suggest the next move as human-readable text.
    pub fn hint_text(&self) -> Option<String> {
        if self.is_won() {
            return None;
        }
        // 1. Safe foundation moves.
        for t in 0..8 {
            if let Some(c) = self.tableau_top(t) {
                if self.is_safe_to_auto(c) && (0..4).any(|f| self.can_place_on_foundation(c, f)) {
                    return Some(format!("Move {} (tableau {}) to foundation.", c, t + 1));
                }
            }
        }
        for c in 0..self.num_cells {
            if let Some(card) = self.cell_card(c) {
                if self.is_safe_to_auto(card)
                    && (0..4).any(|f| self.can_place_on_foundation(card, f))
                {
                    return Some(format!("Move {} (cell {}) to foundation.", card, c + 1));
                }
            }
        }
        // 2. Tableau consolidation (largest fitting run first).
        for from in 0..8 {
            let len = self.tableau[from].len();
            if len == 0 {
                continue;
            }
            for to in 0..8 {
                if to == from {
                    continue;
                }
                let cap = self.max_movable(to).min(len);
                for count in (1..=cap).rev() {
                    let start = len - count;
                    if !Self::is_valid_sequence(&self.tableau[from][start..]) {
                        continue;
                    }
                    let bottom = self.tableau[from][start];
                    if self.can_place_on_tableau(bottom, to) {
                        return Some(format!(
                            "Move {} card(s) from tableau {} to tableau {}.",
                            count,
                            from + 1,
                            to + 1
                        ));
                    }
                }
            }
        }
        // 3. Park a tableau top in a free cell.
        if self.free_count() > 0 {
            for t in 0..8 {
                if let Some(c) = self.tableau_top(t) {
                    return Some(format!("Park {} (tableau {}) in a free cell.", c, t + 1));
                }
            }
        }
        // 4. Play out of a cell onto the tableau.
        for c in 0..self.num_cells {
            if let Some(card) = self.cell_card(c) {
                if let Some(t) = (0..8).find(|&t| self.can_place_on_tableau(card, t)) {
                    return Some(format!(
                        "Move {} (cell {}) to tableau {}.",
                        card,
                        c + 1,
                        t + 1
                    ));
                }
                if (0..4).any(|f| self.can_place_on_foundation(card, f)) {
                    return Some(format!("Move {} (cell {}) to foundation.", card, c + 1));
                }
            }
        }
        None
    }
}

/// Freecell count per difficulty: Easy = 5, Normal = 4, Hard = 3.
pub fn cell_count_for(difficulty: Difficulty) -> usize {
    match difficulty {
        Difficulty::Easy => 5,
        Difficulty::Normal => 4,
        Difficulty::Hard => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{Rank, Suit};

    fn card(suit: Suit, v: u8) -> Card {
        Card::new_face_up(suit, Rank(v))
    }

    #[test]
    fn deal_spreads_52_face_up() {
        let g = FreeCellGame::new();
        let total: usize = g.tableau.iter().map(|p| p.len()).sum();
        assert_eq!(total, 52);
        assert!(g.tableau.iter().flatten().all(|c| c.face_up));
        assert_eq!(g.num_cells, 4);
    }

    #[test]
    fn difficulty_selects_cell_count() {
        assert_eq!(cell_count_for(Difficulty::Easy), 5);
        assert_eq!(cell_count_for(Difficulty::Normal), 4);
        assert_eq!(cell_count_for(Difficulty::Hard), 3);
    }

    #[test]
    fn foundation_rule_suit_ascending() {
        let mut g = FreeCellGame::new();
        let ace = card(Suit::Spades, 1);
        assert!(g.can_place_on_foundation(ace, 0));
        assert!(!g.can_place_on_foundation(card(Suit::Spades, 2), 0));
        g.foundations[0].push(ace);
        assert!(g.can_place_on_foundation(card(Suit::Spades, 2), 0));
        assert!(!g.can_place_on_foundation(card(Suit::Hearts, 2), 0));
    }

    #[test]
    fn tableau_rule_alternating_down() {
        let mut g = FreeCellGame::new();
        for p in &mut g.tableau {
            p.clear();
        }
        g.tableau[0].push(card(Suit::Spades, 7));
        assert!(g.can_place_on_tableau(card(Suit::Hearts, 6), 0));
        assert!(!g.can_place_on_tableau(card(Suit::Clubs, 6), 0));
        assert!(!g.can_place_on_tableau(card(Suit::Hearts, 5), 0));
        assert!(g.can_place_on_tableau(card(Suit::Clubs, 13), 1)); // empty takes any
    }

    #[test]
    fn supermove_capacity_scales() {
        let mut g = FreeCellGame::new();
        // Fill every pile so no column is empty.
        for p in g.tableau.iter_mut() {
            p.clear();
            p.push(card(Suit::Clubs, 13));
        }
        // 4 free cells, no *other* empty columns -> capacity 5.
        assert_eq!(g.max_movable(0), 5);
        // One other empty column doubles it.
        g.tableau[2].clear();
        assert_eq!(g.max_movable(0), 10);
        // The destination itself being empty does not count.
        assert_eq!(g.max_movable(2), 5);
    }

    #[test]
    fn black_four_onto_red_five_is_legal() {
        // Same tableau rule as Klondike (alternating colors, one rank
        // down); empty columns take any card.
        let mut g = FreeCellGame::new();
        for p in &mut g.tableau {
            p.clear();
        }
        g.tableau[0].push(card(Suit::Diamonds, 5));
        g.tableau[1].push(card(Suit::Clubs, 4));
        assert!(g.can_place_on_tableau(card(Suit::Clubs, 4), 0));
        assert!(g.move_tableau_to_tableau(1, 1, 0));
        assert_eq!(g.tableau[0].len(), 2);
        // Same color stays illegal; empty columns still take anything.
        g.tableau[2].push(card(Suit::Clubs, 5));
        assert!(!g.can_place_on_tableau(card(Suit::Spades, 4), 2));
        assert!(g.can_place_on_tableau(card(Suit::Clubs, 4), 3));
    }

    #[test]
    fn safe_auto_moves_aces_then_twos() {
        let mut g = FreeCellGame::new();
        for p in &mut g.tableau {
            p.clear();
        }
        g.tableau[0].push(card(Suit::Spades, 1));
        g.tableau[1].push(card(Suit::Hearts, 1));
        g.tableau[2].push(card(Suit::Spades, 3)); // needs red 2s first: unsafe
        assert_eq!(g.auto_move_safe_to_foundations(), 2); // aces only
        assert_eq!(g.auto_move_all_to_foundations(), 0); // 3 has no ace foundation yet
        g.foundations[0].push(card(Suit::Spades, 1));
        g.foundations[0].push(card(Suit::Spades, 2));
        assert_eq!(g.auto_move_all_to_foundations(), 1); // then the three
    }
}
