//! Full Klondike solitaire state and rules.
//!
//! Layout: 7 tableau piles, 4 foundations, stock + waste. Difficulty sets
//! the default draw count and the redeal limit:
//! - Easy: draw-1, unlimited redeals, and the deal is verified solvable.
//! - Normal: draw-3, unlimited redeals.
//! - Hard: draw-3, at most [`HARD_REDEALS`] stock recycles.

use std::collections::HashSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::cards::{Card, is_valid_descending_alternating_run, shuffled_deck};
use crate::rules::{
    Difficulty, MoveError, Solitaire, check_alternating, foundation_for, is_safe_for_foundation,
};

pub const TABLEAU_PILES: usize = 7;
pub const FOUNDATIONS: usize = 4;

/// Stock recycles allowed on Hard.
pub const HARD_REDEALS: u32 = 3;

// Scoring (Windows "standard" style). Tableau-to-tableau moves score
// nothing, so shuffling cards back and forth can't farm points.
pub const SCORE_WASTE_TO_TABLEAU: i32 = 5;
pub const SCORE_TO_FOUNDATION: i32 = 10;
pub const SCORE_FLIP_TABLEAU: i32 = 5;
pub const SCORE_FOUNDATION_TO_TABLEAU: i32 = -15;
pub const SCORE_REDEAL_PENALTY: i32 = -20;
pub const SCORE_WIN_BONUS: i32 = 500;

/// Easy deals: how many consecutive deal numbers to try, and the search
/// budget per deal, before settling for an unverified deal.
const SOLVABLE_ATTEMPTS: u32 = 40;
const SOLVER_NODE_BUDGET: usize = 4_000;

/// Draw mode for Klondike. `--draw 1` / `--draw 3` on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum DrawMode {
    #[default]
    #[value(name = "1")]
    Draw1,
    #[value(name = "3")]
    Draw3,
}

impl DrawMode {
    pub fn count(self) -> usize {
        match self {
            DrawMode::Draw1 => 1,
            DrawMode::Draw3 => 3,
        }
    }

    /// Default draw count for a difficulty (Easy draws 1, others draw 3).
    pub fn for_difficulty(difficulty: Difficulty) -> Self {
        match difficulty {
            Difficulty::Easy => DrawMode::Draw1,
            Difficulty::Normal | Difficulty::Hard => DrawMode::Draw3,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            DrawMode::Draw1 => DrawMode::Draw3,
            DrawMode::Draw3 => DrawMode::Draw1,
        }
    }
}

impl fmt::Display for DrawMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Draw {}", self.count())
    }
}

/// A move suggested by [`KlondikeGame::hint`] and explored by the solver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    WasteToFoundation,
    WasteToTableau(usize),
    TableauToFoundation(usize),
    TableauToTableau {
        from: usize,
        to: usize,
        count: usize,
    },
    Draw,
    Redeal,
    AutoFinish,
}

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Hint::WasteToFoundation => f.write_str("Move the waste card to a foundation."),
            Hint::WasteToTableau(t) => write!(f, "Move the waste card to tableau pile {}.", t + 1),
            Hint::TableauToFoundation(t) => {
                write!(
                    f,
                    "Move the top card of tableau pile {} to a foundation.",
                    t + 1
                )
            }
            Hint::TableauToTableau { from, to, count } => write!(
                f,
                "Move {} from tableau pile {} to pile {}.",
                crate::rules::cards(count),
                from + 1,
                to + 1
            ),
            Hint::Draw => f.write_str("Draw from the stock."),
            Hint::Redeal => f.write_str("Recycle the waste back into the stock."),
            Hint::AutoFinish => f.write_str("Everything is face up: press a to finish."),
        }
    }
}

/// Full Klondike game state.
#[derive(Debug, Clone)]
pub struct KlondikeGame {
    pub draw_mode: DrawMode,
    pub difficulty: Difficulty,
    pub stock: Vec<Card>,
    pub waste: Vec<Card>,
    pub foundations: [Vec<Card>; FOUNDATIONS],
    pub tableau: [Vec<Card>; TABLEAU_PILES],
    score: i32,
    moves: u32,
    redeals_used: u32,
    deal: u32,
}

impl KlondikeGame {
    /// Deal number `deal` with the given rules.
    pub fn new(draw_mode: DrawMode, difficulty: Difficulty, deal: u32) -> Self {
        let mut game = Self {
            draw_mode,
            difficulty,
            stock: Vec::new(),
            waste: Vec::new(),
            foundations: Default::default(),
            tableau: Default::default(),
            score: 0,
            moves: 0,
            redeals_used: 0,
            deal,
        };
        game.restart();
        game
    }

    /// The first deal at or after `first_deal` that the solver can win.
    /// Used for Easy; falls back to an unverified deal if the search budget
    /// runs out (rare).
    pub fn new_solvable(draw_mode: DrawMode, difficulty: Difficulty, first_deal: u32) -> Self {
        (0..SOLVABLE_ATTEMPTS)
            .map(|k| Self::new(draw_mode, difficulty, first_deal.wrapping_add(k)))
            .find(|g| g.is_solvable(SOLVER_NODE_BUDGET))
            .unwrap_or_else(|| Self::new(draw_mode, difficulty, first_deal))
    }

    /// Re-deal the same deal number from scratch.
    pub fn restart(&mut self) {
        let mut deck = shuffled_deck(self.deal);
        for (i, pile) in self.tableau.iter_mut().enumerate() {
            pile.clear();
            pile.extend(deck.drain(deck.len() - (i + 1)..));
            for c in pile.iter_mut() {
                c.face_up = false;
            }
            if let Some(top) = pile.last_mut() {
                top.face_up = true;
            }
        }
        self.stock = deck;
        self.waste.clear();
        for f in &mut self.foundations {
            f.clear();
        }
        self.score = 0;
        self.moves = 0;
        self.redeals_used = 0;
    }

    // ---- accessors ----

    /// Redeal limit: `None` = unlimited (Easy/Normal).
    pub fn redeal_limit(&self) -> Option<u32> {
        (self.difficulty == Difficulty::Hard).then_some(HARD_REDEALS)
    }

    pub fn redeals_remaining(&self) -> Option<u32> {
        self.redeal_limit()
            .map(|limit| limit.saturating_sub(self.redeals_used))
    }

    pub fn can_redeal(&self) -> bool {
        self.redeals_remaining() != Some(0)
    }

    pub fn waste_top(&self) -> Option<Card> {
        self.waste.last().copied()
    }

    pub fn tableau_top(&self, pile: usize) -> Option<Card> {
        self.tableau.get(pile)?.last().copied()
    }

    pub fn foundation_top(&self, pile: usize) -> Option<Card> {
        self.foundations.get(pile)?.last().copied()
    }

    // ---- stock / waste ----

    /// Draw `draw_count` cards stock -> waste, or recycle the waste when the
    /// stock is empty. Returns false when nothing happened (won, nothing to
    /// draw, or Hard-mode redeals used up).
    pub fn draw_from_stock(&mut self) -> bool {
        if self.is_won() {
            return false;
        }
        if self.stock.is_empty() {
            if self.waste.is_empty() || !self.can_redeal() {
                return false;
            }
            self.stock.extend(self.waste.drain(..).rev().map(|mut c| {
                c.face_up = false;
                c
            }));
            self.redeals_used += 1;
            self.moves += 1;
            self.add_score(SCORE_REDEAL_PENALTY);
            return true;
        }
        let n = self.draw_mode.count().min(self.stock.len());
        let start = self.stock.len() - n;
        self.waste
            .extend(self.stock.drain(start..).rev().map(|mut c| {
                c.face_up = true;
                c
            }));
        self.moves += 1;
        true
    }

    // ---- placement rules ----

    /// Tableau rule: empty takes a King; otherwise alternating colors, one
    /// rank lower.
    pub fn check_tableau(&self, card: Card, tableau: usize) -> Result<(), MoveError> {
        let pile = self.tableau.get(tableau).ok_or(MoveError::NotAllowed)?;
        match pile.last() {
            None if card.rank.is_king() => Ok(()),
            None => Err(MoveError::EmptyColumnNeedsKing),
            Some(&top) => check_alternating(top, card),
        }
    }

    pub fn can_place_on_tableau(&self, card: Card, tableau: usize) -> bool {
        self.check_tableau(card, tableau).is_ok()
    }

    /// Foundation that accepts `card`, if any.
    pub fn foundation_for(&self, card: Card) -> Option<usize> {
        foundation_for(&self.foundations, card)
    }

    /// Index of the first face-up card in a tableau pile.
    fn first_face_up(&self, pile: usize) -> Option<usize> {
        self.tableau.get(pile)?.iter().position(|c| c.face_up)
    }

    /// Largest grabbable suffix of a tableau pile (the face-up run).
    pub fn grab_count_tableau(&self, pile: usize) -> usize {
        let Some(first) = self.first_face_up(pile) else {
            return 0;
        };
        let p = &self.tableau[pile];
        (first..p.len())
            .find(|&start| is_valid_descending_alternating_run(&p[start..]))
            .map_or(0, |start| p.len() - start)
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
        self.check_tableau(pile[start], to)
    }

    /// Flip a newly exposed tableau top face-up (+score).
    fn reveal_tableau_top(&mut self, pile: usize) {
        if let Some(top) = self.tableau[pile].last_mut()
            && !top.face_up
        {
            top.face_up = true;
            self.score += SCORE_FLIP_TABLEAU;
        }
    }

    fn add_score(&mut self, delta: i32) {
        self.score = (self.score + delta).max(0);
    }

    fn on_card_moved_to_foundation(&mut self) {
        self.add_score(SCORE_TO_FOUNDATION);
        self.moves += 1;
        if self.is_won() {
            self.add_score(SCORE_WIN_BONUS);
        }
    }

    // ---- moves ----

    /// Waste top -> the foundation that accepts it.
    pub fn move_waste_to_foundation(&mut self) -> bool {
        let Some(card) = self.waste_top() else {
            return false;
        };
        let Some(f) = self.foundation_for(card) else {
            return false;
        };
        self.waste.pop();
        self.foundations[f].push(card);
        self.on_card_moved_to_foundation();
        true
    }

    /// Waste top -> tableau pile.
    pub fn move_waste_to_tableau(&mut self, tableau: usize) -> bool {
        let Some(card) = self.waste_top() else {
            return false;
        };
        if !self.can_place_on_tableau(card, tableau) {
            return false;
        }
        self.waste.pop();
        self.tableau[tableau].push(card);
        self.add_score(SCORE_WASTE_TO_TABLEAU);
        self.moves += 1;
        true
    }

    /// Tableau pile top -> the foundation that accepts it.
    pub fn move_tableau_to_foundation(&mut self, tableau: usize) -> bool {
        let Some(card) = self.tableau_top(tableau).filter(|c| c.face_up) else {
            return false;
        };
        let Some(f) = self.foundation_for(card) else {
            return false;
        };
        self.tableau[tableau].pop();
        self.foundations[f].push(card);
        self.reveal_tableau_top(tableau);
        self.on_card_moved_to_foundation();
        true
    }

    /// Move exactly the top `count` cards of one tableau pile to another.
    pub fn move_tableau_to_tableau(&mut self, from: usize, to: usize, count: usize) -> bool {
        if self.check_tableau_move(from, to, count).is_err() {
            return false;
        }
        let start = self.tableau[from].len() - count;
        let moving: Vec<Card> = self.tableau[from].drain(start..).collect();
        self.tableau[to].extend(moving);
        self.reveal_tableau_top(from);
        self.moves += 1;
        true
    }

    /// Foundation top -> tableau pile (costs points).
    pub fn move_foundation_to_tableau(&mut self, foundation: usize, tableau: usize) -> bool {
        let Some(card) = self.foundation_top(foundation) else {
            return false;
        };
        if !self.can_place_on_tableau(card, tableau) {
            return false;
        }
        self.foundations[foundation].pop();
        self.tableau[tableau].push(card);
        self.add_score(SCORE_FOUNDATION_TO_TABLEAU);
        self.moves += 1;
        true
    }

    /// Move every *safe* card (see [`is_safe_for_foundation`]) from the
    /// waste and tableau tops to the foundations. Returns cards moved.
    pub fn auto_move_safe(&mut self) -> usize {
        let mut moved = 0;
        loop {
            let safe = |c: Card| is_safe_for_foundation(&self.foundations, c);
            let waste_safe = self.waste_top().is_some_and(safe);
            if waste_safe && self.move_waste_to_foundation() {
                moved += 1;
                continue;
            }
            let pile = (0..TABLEAU_PILES).find(|&t| {
                self.tableau_top(t).is_some_and(|c| {
                    c.face_up
                        && is_safe_for_foundation(&self.foundations, c)
                        && self.foundation_for(c).is_some()
                })
            });
            match pile {
                Some(t) if self.move_tableau_to_foundation(t) => moved += 1,
                _ => return moved,
            }
        }
    }

    /// True when stock and waste are empty and every tableau card is face
    /// up: every remaining card can then go up in order.
    pub fn can_auto_finish(&self) -> bool {
        !self.is_won()
            && self.stock.is_empty()
            && self.waste.is_empty()
            && self.tableau.iter().flatten().all(|c| c.face_up)
    }

    /// Move everything to the foundations (only when [`can_auto_finish`]).
    ///
    /// [`can_auto_finish`]: KlondikeGame::can_auto_finish
    pub fn auto_finish(&mut self) -> usize {
        if !self.can_auto_finish() {
            return 0;
        }
        let mut moved = 0;
        while let Some(t) = (0..TABLEAU_PILES).find(|&t| {
            self.tableau_top(t)
                .is_some_and(|c| self.foundation_for(c).is_some())
        }) {
            self.move_tableau_to_foundation(t);
            moved += 1;
        }
        moved
    }

    // ---- hints / solver ----

    /// Every move that makes progress, best first. Tableau moves are only
    /// offered when they expose a face-down card, empty a column for a
    /// waiting King, or free a card for the foundations, so following
    /// these moves can never ping-pong. Drawing is only offered while some
    /// stock/waste card could actually be played.
    pub fn progress_moves(&self) -> Vec<Hint> {
        let mut out = Vec::new();
        if self.is_won() {
            return out;
        }
        if self
            .waste_top()
            .is_some_and(|w| self.foundation_for(w).is_some())
        {
            out.push(Hint::WasteToFoundation);
        }
        for t in 0..TABLEAU_PILES {
            if self
                .tableau_top(t)
                .is_some_and(|c| c.face_up && self.foundation_for(c).is_some())
            {
                out.push(Hint::TableauToFoundation(t));
            }
        }

        // A King-headed run sitting on other cards, or a King in the waste,
        // could use an emptied column.
        let king_waiting = self.waste_top().is_some_and(|c| c.rank.is_king())
            || (0..TABLEAU_PILES).any(|t| {
                self.first_face_up(t)
                    .is_some_and(|first| first > 0 && self.tableau[t][first].rank.is_king())
            });
        for from in 0..TABLEAU_PILES {
            let Some(first) = self.first_face_up(from) else {
                continue;
            };
            let len = self.tableau[from].len();
            for start in first..len {
                let count = len - start;
                let useful = if start == first {
                    first > 0 || king_waiting
                } else {
                    self.foundation_for(self.tableau[from][start - 1]).is_some()
                };
                if !useful {
                    continue;
                }
                for to in (0..TABLEAU_PILES).filter(|&to| to != from) {
                    // Moving a whole column into an empty one changes nothing.
                    let pointless = start == 0 && self.tableau[to].is_empty();
                    if !pointless && self.check_tableau_move(from, to, count).is_ok() {
                        out.push(Hint::TableauToTableau { from, to, count });
                    }
                }
            }
        }

        if let Some(w) = self.waste_top() {
            out.extend(
                (0..TABLEAU_PILES)
                    .filter(|&t| self.can_place_on_tableau(w, t))
                    .map(Hint::WasteToTableau),
            );
        }

        let can_cycle = !self.stock.is_empty() || (!self.waste.is_empty() && self.can_redeal());
        let playable = |c: &Card| {
            self.foundation_for(*c).is_some()
                || (0..TABLEAU_PILES).any(|t| self.can_place_on_tableau(*c, t))
        };
        if can_cycle && self.stock.iter().chain(&self.waste).any(playable) {
            out.push(if self.stock.is_empty() {
                Hint::Redeal
            } else {
                Hint::Draw
            });
        }
        out
    }

    /// Carry out a suggested move.
    pub fn apply(&mut self, hint: Hint) -> bool {
        match hint {
            Hint::WasteToFoundation => self.move_waste_to_foundation(),
            Hint::WasteToTableau(t) => self.move_waste_to_tableau(t),
            Hint::TableauToFoundation(t) => self.move_tableau_to_foundation(t),
            Hint::TableauToTableau { from, to, count } => {
                self.move_tableau_to_tableau(from, to, count)
            }
            Hint::Draw | Hint::Redeal => self.draw_from_stock(),
            Hint::AutoFinish => self.auto_finish() > 0,
        }
    }

    /// Best next move, if any.
    pub fn best_move(&self) -> Option<Hint> {
        if self.can_auto_finish() {
            return Some(Hint::AutoFinish);
        }
        self.progress_moves().into_iter().next()
    }

    fn state_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.stock.hash(&mut h);
        self.waste.hash(&mut h);
        self.tableau.hash(&mut h);
        for f in &self.foundations {
            f.len().hash(&mut h);
        }
        h.finish()
    }

    /// Depth-first search over [`progress_moves`] with safe foundation moves
    /// applied eagerly. `true` means a win was found within `budget` nodes;
    /// `false` means unknown (or unwinnable).
    ///
    /// [`progress_moves`]: KlondikeGame::progress_moves
    pub fn is_solvable(&self, budget: usize) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![self.clone()];
        while let Some(mut game) = stack.pop() {
            game.auto_move_safe();
            if game.is_won() || game.can_auto_finish() {
                return true;
            }
            if !seen.insert(game.state_key()) {
                continue;
            }
            if seen.len() > budget {
                return false;
            }
            for hint in game.progress_moves().into_iter().rev() {
                let mut child = game.clone();
                if child.apply(hint) {
                    stack.push(child);
                }
            }
        }
        false
    }
}

impl Solitaire for KlondikeGame {
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
        self.best_move().map(|h| h.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{Rank, Suit};

    fn up(suit: Suit, v: u8) -> Card {
        Card::up(suit, v)
    }

    fn empty_game(difficulty: Difficulty) -> KlondikeGame {
        let mut g = KlondikeGame::new(DrawMode::Draw1, difficulty, 1);
        g.stock.clear();
        g.waste.clear();
        g.tableau.iter_mut().for_each(Vec::clear);
        g
    }

    fn all_52_unique(game: &KlondikeGame) -> bool {
        let cards: Vec<_> = game
            .stock
            .iter()
            .chain(&game.waste)
            .chain(game.foundations.iter().flatten())
            .chain(game.tableau.iter().flatten())
            .map(|c| (c.suit, c.rank))
            .collect();
        let unique: HashSet<_> = cards.iter().collect();
        cards.len() == 52 && unique.len() == 52
    }

    #[test]
    fn deal_layout_is_valid() {
        let game = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal, 5);
        for (i, pile) in game.tableau.iter().enumerate() {
            assert_eq!(pile.len(), i + 1, "pile {i} size");
            assert!(pile.last().unwrap().face_up, "pile {i} top face-up");
            assert!(
                pile[..i].iter().all(|c| !c.face_up),
                "pile {i} fillers face-down"
            );
        }
        assert_eq!(game.stock.len(), 24);
        assert_eq!(game.waste, []);
        assert!(game.foundations.iter().all(Vec::is_empty));
        assert!(all_52_unique(&game));
        assert_eq!(game.score(), 0);
        assert_eq!(game.move_count(), 0);
    }

    #[test]
    fn restart_reproduces_the_deal() {
        let mut game = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal, 99);
        let tableau = game.tableau.clone();
        let stock = game.stock.clone();
        game.draw_from_stock();
        game.restart();
        assert_eq!(game.tableau, tableau);
        assert_eq!(game.stock, stock);
        assert_eq!(game.move_count(), 0);
    }

    #[test]
    fn draw_modes_and_recycle() {
        let mut g1 = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy, 1);
        assert!(g1.draw_from_stock());
        assert_eq!((g1.waste.len(), g1.stock.len()), (1, 23));

        let mut g3 = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal, 1);
        let next_three: Vec<Card> = g3.stock[21..].iter().rev().copied().collect();
        assert!(g3.draw_from_stock());
        assert_eq!((g3.waste.len(), g3.stock.len()), (3, 21));
        // The stock top is drawn first and ends up underneath.
        assert_eq!(g3.waste[0].rank, next_three[0].rank);

        while !g3.stock.is_empty() {
            assert!(g3.draw_from_stock());
        }
        let order: Vec<Card> = g3.waste.clone();
        assert!(g3.draw_from_stock()); // recycle
        assert_eq!(g3.stock.len(), 24);
        assert_eq!(g3.waste, []);
        // Recycling preserves draw order.
        assert_eq!(g3.stock.last().unwrap().rank, order[0].rank);
    }

    #[test]
    fn hard_mode_refuses_extra_redeals_without_ending_the_game() {
        let mut game = KlondikeGame::new(DrawMode::Draw3, Difficulty::Hard, 3);
        assert_eq!(game.redeals_remaining(), Some(HARD_REDEALS));
        for expected in (0..HARD_REDEALS).rev() {
            while !game.stock.is_empty() {
                assert!(game.draw_from_stock());
            }
            assert!(game.draw_from_stock(), "redeal should succeed");
            assert_eq!(game.redeals_remaining(), Some(expected));
        }
        while !game.stock.is_empty() {
            assert!(game.draw_from_stock());
        }
        assert!(!game.can_redeal());
        assert!(!game.draw_from_stock(), "extra redeal refused");
        // Tableau play continues after the stock runs out.
        game.tableau.iter_mut().for_each(Vec::clear);
        game.tableau[0].push(up(Suit::Spades, 13));
        game.tableau[1].push(up(Suit::Hearts, 12));
        assert!(game.move_tableau_to_tableau(1, 0, 1));
    }

    #[test]
    fn easy_and_normal_have_unlimited_redeals() {
        let easy = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy, 1);
        let normal = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal, 1);
        assert_eq!(easy.redeal_limit(), None);
        assert_eq!(normal.redeal_limit(), None);
        assert!(easy.can_redeal() && normal.can_redeal());
    }

    #[test]
    fn foundation_rules() {
        let mut game = empty_game(Difficulty::Easy);
        let ace = up(Suit::Spades, 1);
        assert_eq!(game.foundation_for(ace), Some(0));
        assert_eq!(game.foundation_for(up(Suit::Spades, 2)), None);
        game.foundations[2].push(ace);
        assert_eq!(game.foundation_for(up(Suit::Spades, 2)), Some(2));
        assert_eq!(game.foundation_for(up(Suit::Hearts, 2)), None);
    }

    #[test]
    fn tableau_rules_alternating_descending() {
        let mut g = empty_game(Difficulty::Easy);
        let king = up(Suit::Spades, 13);
        assert!(g.can_place_on_tableau(king, 0));
        assert_eq!(
            g.check_tableau(up(Suit::Hearts, 12), 0),
            Err(MoveError::EmptyColumnNeedsKing)
        );
        g.tableau[0].push(king);
        assert!(g.can_place_on_tableau(up(Suit::Hearts, 12), 0));
        assert_eq!(
            g.check_tableau(up(Suit::Spades, 12), 0),
            Err(MoveError::SameColor)
        );
        assert_eq!(
            g.check_tableau(up(Suit::Hearts, 11), 0),
            Err(MoveError::WrongRank)
        );
        assert!(!g.can_place_on_tableau(king, 9));
        g.tableau[4].push(Card::new(Suit::Diamonds, Rank::new(5)));
        assert!(
            !g.can_place_on_tableau(up(Suit::Clubs, 4), 4),
            "face-down top"
        );
    }

    #[test]
    fn black_four_onto_red_five_is_legal() {
        let mut game = empty_game(Difficulty::Easy);
        game.tableau[0].push(up(Suit::Diamonds, 5));
        game.tableau[1].push(up(Suit::Clubs, 4));
        assert!(game.move_tableau_to_tableau(1, 0, 1));
        assert_eq!(game.tableau[0].len(), 2);
        game.tableau[2].push(up(Suit::Diamonds, 5));
        game.waste.push(up(Suit::Clubs, 4));
        assert!(game.move_waste_to_tableau(2));
    }

    #[test]
    fn waste_and_tableau_to_foundation_score() {
        let mut game = empty_game(Difficulty::Easy);
        game.waste.push(up(Suit::Clubs, 1));
        assert!(game.move_waste_to_foundation());
        assert_eq!(game.score(), SCORE_TO_FOUNDATION);

        game.tableau[0].push(Card::new(Suit::Hearts, Rank::new(5)));
        game.tableau[0].push(up(Suit::Clubs, 2));
        assert!(game.move_tableau_to_foundation(0));
        assert!(game.tableau[0][0].face_up, "exposed card flips");
        assert_eq!(game.score(), 2 * SCORE_TO_FOUNDATION + SCORE_FLIP_TABLEAU);
    }

    #[test]
    fn multi_card_move_and_tableau_moves_score_nothing() {
        let mut game = empty_game(Difficulty::Easy);
        game.tableau[0].push(Card::new(Suit::Spades, Rank::ACE));
        game.tableau[0].extend([up(Suit::Clubs, 9), up(Suit::Hearts, 8), up(Suit::Clubs, 7)]);
        game.tableau[1].push(up(Suit::Hearts, 10));
        assert!(game.move_tableau_to_tableau(0, 1, 3));
        assert_eq!(game.tableau[1].len(), 4);
        assert!(game.tableau[0][0].face_up);
        assert_eq!(game.score(), SCORE_FLIP_TABLEAU, "only the flip scores");

        // Ping-ponging a card between two equal targets earns nothing.
        let mut g = empty_game(Difficulty::Easy);
        g.tableau[0].extend([up(Suit::Clubs, 9), up(Suit::Hearts, 8)]);
        g.tableau[1].push(up(Suit::Spades, 9));
        for _ in 0..3 {
            assert!(g.move_tableau_to_tableau(0, 1, 1));
            assert!(g.move_tableau_to_tableau(1, 0, 1));
        }
        assert_eq!(g.score(), 0);

        let mut bad = empty_game(Difficulty::Easy);
        bad.tableau[0].extend([up(Suit::Clubs, 9), up(Suit::Spades, 8)]);
        bad.tableau[1].push(up(Suit::Hearts, 10));
        assert_eq!(bad.check_tableau_move(0, 1, 2), Err(MoveError::NotARun));
    }

    #[test]
    fn foundation_to_tableau_costs_points() {
        let mut game = empty_game(Difficulty::Easy);
        game.foundations[0].extend([up(Suit::Spades, 1), up(Suit::Spades, 2)]);
        game.tableau[0].push(up(Suit::Hearts, 3));
        game.score = 50;
        assert!(game.move_foundation_to_tableau(0, 0));
        assert_eq!(game.score(), 50 + SCORE_FOUNDATION_TO_TABLEAU);
    }

    #[test]
    fn win_detection_and_bonus() {
        let mut game = empty_game(Difficulty::Easy);
        for (i, &suit) in Suit::ALL.iter().enumerate() {
            game.foundations[i] = Rank::all()
                .filter(|r| !(suit == Suit::Clubs && r.is_king()))
                .map(|r| up(suit, r.value()))
                .collect();
        }
        game.waste.push(up(Suit::Clubs, 13));
        assert!(!game.is_won());
        assert!(game.move_waste_to_foundation());
        assert!(game.is_won());
        assert_eq!(game.score(), SCORE_TO_FOUNDATION + SCORE_WIN_BONUS);
    }

    #[test]
    fn auto_move_only_takes_safe_cards() {
        let mut game = empty_game(Difficulty::Easy);
        game.foundations[0].extend([up(Suit::Hearts, 1), up(Suit::Hearts, 2)]);
        game.foundations[1].extend([up(Suit::Spades, 1), up(Suit::Spades, 2)]);
        game.tableau[0].push(up(Suit::Spades, 3)); // 2♦ may still need it
        game.tableau[1].push(up(Suit::Clubs, 1));
        assert_eq!(game.auto_move_safe(), 1, "only the ace");
        assert_eq!(game.tableau[0].len(), 1);
    }

    #[test]
    fn auto_finish_requires_everything_face_up() {
        let mut game = empty_game(Difficulty::Easy);
        game.tableau[0].extend([up(Suit::Spades, 2), up(Suit::Hearts, 1)]);
        game.tableau[1].push(up(Suit::Spades, 1));
        assert!(game.can_auto_finish());
        assert_eq!(game.best_move(), Some(Hint::AutoFinish));
        assert_eq!(game.auto_finish(), 3);

        let mut hidden = empty_game(Difficulty::Easy);
        hidden.tableau[0].push(Card::new(Suit::Spades, Rank::ACE));
        hidden.tableau[0].push(up(Suit::Hearts, 1));
        assert!(!hidden.can_auto_finish());
        assert_eq!(hidden.auto_finish(), 0);
    }

    #[test]
    fn hints_never_ping_pong() {
        // 8♥ could bounce between 9♣ and 9♠ forever; neither move exposes
        // anything, so the hint must not suggest it.
        let mut g = empty_game(Difficulty::Hard);
        g.tableau[0] = vec![
            Card::new(Suit::Diamonds, Rank::KING),
            up(Suit::Clubs, 9),
            up(Suit::Hearts, 8),
        ];
        g.tableau[1] = vec![
            Card::new(Suit::Diamonds, Rank::new(12)),
            up(Suit::Spades, 9),
        ];
        assert_eq!(g.best_move(), None);
        assert!(g.is_stuck());

        // Moving the whole face-up run does expose a card: suggested.
        g.tableau[2].push(up(Suit::Hearts, 10));
        assert_eq!(
            g.best_move(),
            Some(Hint::TableauToTableau {
                from: 0,
                to: 2,
                count: 2
            })
        );
    }

    #[test]
    fn hint_draws_only_when_a_stock_card_is_playable() {
        let mut g = empty_game(Difficulty::Normal);
        g.tableau[0].push(up(Suit::Spades, 9));
        g.stock.push(Card::new(Suit::Clubs, Rank::new(5)));
        assert_eq!(g.best_move(), None, "5♣ fits nowhere: drawing is pointless");
        g.stock.push(Card::new(Suit::Hearts, Rank::new(8)));
        assert_eq!(g.best_move(), Some(Hint::Draw));
    }

    #[test]
    fn following_hints_terminates() {
        for deal in 0..20 {
            let mut g = KlondikeGame::new(DrawMode::Draw3, Difficulty::Hard, deal);
            let mut steps = 0;
            while let Some(h) = g.best_move() {
                assert!(g.apply(h), "hint {h:?} must be legal");
                steps += 1;
                assert!(steps < 2_000, "deal {deal}: hints looped");
            }
        }
    }

    #[test]
    fn easy_deals_are_solvable_and_varied() {
        let a = KlondikeGame::new_solvable(DrawMode::Draw1, Difficulty::Easy, 1_000);
        let b = KlondikeGame::new_solvable(DrawMode::Draw1, Difficulty::Easy, 2_000);
        assert!(a.is_solvable(SOLVER_NODE_BUDGET));
        assert!(b.is_solvable(SOLVER_NODE_BUDGET));
        assert_ne!(a.tableau, b.tableau, "different seeds give different deals");
        assert!(all_52_unique(&a));
    }

    #[test]
    fn solver_wins_a_trivial_position() {
        let mut g = empty_game(Difficulty::Easy);
        g.tableau[0].extend([up(Suit::Spades, 2), up(Suit::Hearts, 1)]);
        g.stock.push(Card::new(Suit::Spades, Rank::ACE));
        assert!(g.is_solvable(100));
    }
}
