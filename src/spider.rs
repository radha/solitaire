//! Simplified Spider (Spiderette / Spider-mini) state and rules.
//!
//! Layout: 10 tableau piles, a stock that deals one card to every pile,
//! and one foundation slot per completed sequence.
//! - Tableau builds down by rank regardless of suit; any card (run) may
//!   occupy an empty column.
//! - Only same-suit descending runs may move together.
//! - A complete K -> A same-suit run (13 cards) flies to the foundation.
//! - The stock deals 10 cards at a time and refuses while any column is
//!   empty (classic rule). 5 deals total from a 104-card deck.
//! - Difficulty = suits in play: Easy = 1, Normal = 2, Hard = 4.

use rand::seq::SliceRandom;
use rand::thread_rng;

use crate::cards::{Card, Rank, Suit};
use crate::klondike::Difficulty;

/// Suits in play (difficulty selector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiderSuits {
    /// Easy: 1 suit (8 identical-suit sets).
    #[default]
    One,
    /// Normal: 2 suits (4 sets each).
    Two,
    /// Hard: 4 suits (2 sets each).
    Four,
}

impl SpiderSuits {
    pub fn from_difficulty(d: Difficulty) -> Self {
        match d {
            Difficulty::Easy => SpiderSuits::One,
            Difficulty::Normal => SpiderSuits::Two,
            Difficulty::Hard => SpiderSuits::Four,
        }
    }

    pub fn suits_in_play(self) -> Vec<Suit> {
        match self {
            SpiderSuits::One => vec![Suit::Spades],
            SpiderSuits::Two => vec![Suit::Spades, Suit::Hearts],
            SpiderSuits::Four => vec![Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SpiderSuits::One => "1 suit",
            SpiderSuits::Two => "2 suits",
            SpiderSuits::Four => "4 suits",
        }
    }

    /// Copies of each rank set needed for a 104-card deck.
    fn copies_per_suit(self) -> usize {
        match self {
            SpiderSuits::One => 8,
            SpiderSuits::Two => 4,
            SpiderSuits::Four => 2,
        }
    }
}

pub const SCORE_SEQUENCE_COMPLETE: i32 = 100;
pub const SCORE_TABLEAU_MOVE: i32 = 3;
pub const SCORE_WIN_BONUS: i32 = 500;
pub const SEQUENCES_TO_WIN: u8 = 8;

/// Simplified Spider game state.
#[derive(Debug, Clone)]
pub struct SpiderGame {
    pub suits: SpiderSuits,
    pub tableau: [Vec<Card>; 10],
    pub stock: Vec<Card>,
    pub completed_sequences: u8,
    score: i32,
    moves: u32,
    win_bonus_awarded: bool,
}

impl Default for SpiderGame {
    fn default() -> Self {
        Self::new(SpiderSuits::One)
    }
}

impl SpiderGame {
    pub fn new(suits: SpiderSuits) -> Self {
        let mut game = Self {
            suits,
            tableau: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            stock: Vec::new(),
            completed_sequences: 0,
            score: 0,
            moves: 0,
            win_bonus_awarded: false,
        };
        game.reset();
        game
    }

    /// Fresh deal: 54 cards to the tableau (piles 0-3 get 6, piles 4-9 get
    /// 5), tops face-up, remaining 50 form the stock (5 deals of 10).
    pub fn reset(&mut self) {
        let mut deck = build_deck(self.suits);
        deck.shuffle(&mut thread_rng());
        for pile in &mut self.tableau {
            pile.clear();
        }
        for (i, pile) in self.tableau.iter_mut().enumerate() {
            let n = if i < 4 { 6 } else { 5 };
            for _ in 0..n {
                if let Some(mut c) = deck.pop() {
                    c.face_up = false;
                    pile.push(c);
                }
            }
            if let Some(top) = pile.last_mut() {
                top.face_up = true;
            }
        }
        for c in deck.iter_mut() {
            c.face_up = false;
        }
        self.stock = deck;
        self.completed_sequences = 0;
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

    pub fn stock_deals_remaining(&self) -> usize {
        self.stock.len() / 10
    }

    pub fn tableau_top(&self, pile: usize) -> Option<Card> {
        self.tableau.get(pile)?.last().copied()
    }

    pub fn is_won(&self) -> bool {
        self.completed_sequences >= SEQUENCES_TO_WIN
    }

    // ---- rules ----

    /// Tableau rule: empty takes anything; otherwise one rank lower, any suit.
    pub fn can_place_on_tableau(&self, card: Card, to: usize) -> bool {
        let pile = match self.tableau.get(to) {
            Some(p) => p,
            None => return false,
        };
        match pile.last() {
            None => true,
            Some(top) => top.face_up && top.rank.value() == card.rank.value() + 1,
        }
    }

    /// Length of the same-suit descending face-up run ending at the top.
    /// This is the most cards that may move together from `pile`.
    pub fn movable_run_len(&self, pile: usize) -> usize {
        let p = match self.tableau.get(pile) {
            Some(p) => p,
            None => return 0,
        };
        if p.is_empty() || !p.last().map(|c| c.face_up).unwrap_or(false) {
            return 0;
        }
        let mut run = 1;
        let mut i = p.len() - 1;
        while i > 0 {
            let (below, above) = (p[i - 1], p[i]);
            if below.face_up
                && below.suit == above.suit
                && below.rank.value() == above.rank.value() + 1
            {
                run += 1;
                i -= 1;
            } else {
                break;
            }
        }
        run
    }

    fn flip_top(&mut self, pile: usize) {
        if let Some(top) = self.tableau.get_mut(pile).and_then(|p| p.last_mut()) {
            top.face_up = true;
        }
    }

    /// Remove a complete K..A same-suit run ending at the pile top.
    /// Returns true when a sequence was banked.
    pub fn remove_completed_at(&mut self, pile: usize) -> bool {
        if pile >= 10 {
            return false;
        }
        let len = self.tableau[pile].len();
        if len < 13 {
            return false;
        }
        let start = len - 13;
        let suit = self.tableau[pile][start].suit;
        if !self.tableau[pile][start].face_up || self.tableau[pile][start].rank.value() != 13 {
            return false;
        }
        for (k, c) in self.tableau[pile][start..].iter().enumerate() {
            if !c.face_up || c.suit != suit || c.rank.value() != 13 - k as u8 {
                return false;
            }
        }
        self.tableau[pile].drain(start..);
        self.completed_sequences += 1;
        self.score += SCORE_SEQUENCE_COMPLETE;
        self.moves += 1;
        self.flip_top(pile);
        if self.is_won() && !self.win_bonus_awarded {
            self.win_bonus_awarded = true;
            self.score += SCORE_WIN_BONUS;
        }
        true
    }

    /// Scan every pile for completable runs. Returns sequences banked.
    pub fn auto_remove_completed(&mut self) -> usize {
        let mut n = 0;
        for p in 0..10 {
            if self.remove_completed_at(p) {
                n += 1;
            }
        }
        n
    }

    /// Move the top `count` cards between tableau piles. Over-long requests
    /// shrink to the largest fitting same-suit run.
    pub fn move_tableau_to_tableau(&mut self, from: usize, mut count: usize, to: usize) -> bool {
        if from >= 10 || to >= 10 || from == to || count == 0 {
            return false;
        }
        let len = self.tableau[from].len();
        if len == 0 {
            return false;
        }
        count = count.min(len).min(self.movable_run_len(from));
        while count > 0 {
            let start = len - count;
            if self.can_place_on_tableau(self.tableau[from][start], to) {
                let mut moving: Vec<Card> = self.tableau[from].drain(start..).collect();
                self.tableau[to].append(&mut moving);
                self.flip_top(from);
                self.score += SCORE_TABLEAU_MOVE;
                self.moves += 1;
                self.remove_completed_at(to);
                return true;
            }
            count -= 1;
        }
        false
    }

    /// Whether the stock may be dealt now (needs 10 cards and no empty
    /// column, per the classic rule).
    pub fn can_deal(&self) -> bool {
        self.stock.len() >= 10 && self.tableau.iter().all(|p| !p.is_empty())
    }

    /// Deal one face-up card to each tableau pile. Returns false when the
    /// stock is short or a column is empty.
    pub fn deal_from_stock(&mut self) -> bool {
        if !self.can_deal() {
            return false;
        }
        for pile in &mut self.tableau {
            if let Some(mut c) = self.stock.pop() {
                c.face_up = true;
                pile.push(c);
            }
        }
        self.moves += 1;
        self.auto_remove_completed();
        true
    }

    /// True when at least one legal action exists (a tableau move or a deal).
    pub fn has_legal_moves(&self) -> bool {
        if self.is_won() {
            return false;
        }
        if self.can_deal() {
            return true;
        }
        for from in 0..10 {
            let run = self.movable_run_len(from);
            if run == 0 {
                continue;
            }
            let len = self.tableau[from].len();
            for count in 1..=run {
                let bottom = self.tableau[from][len - count];
                if (0..10).any(|to| to != from && self.can_place_on_tableau(bottom, to)) {
                    return true;
                }
            }
        }
        false
    }

    /// Suggest the next move as human-readable text.
    pub fn hint_text(&self) -> Option<String> {
        if self.is_won() {
            return None;
        }
        // 1. Moves that extend a same-suit run (join matching suit).
        for from in 0..10 {
            let run = self.movable_run_len(from);
            if run == 0 {
                continue;
            }
            let len = self.tableau[from].len();
            for count in (1..=run).rev() {
                let bottom = self.tableau[from][len - count];
                for to in 0..10 {
                    if to == from {
                        continue;
                    }
                    if !self.can_place_on_tableau(bottom, to) {
                        continue;
                    }
                    let same_suit_join = self.tableau[to]
                        .last()
                        .map(|t| t.suit == bottom.suit)
                        .unwrap_or(true);
                    if same_suit_join {
                        return Some(format!(
                            "Move {} (tableau {}) onto tableau {}.",
                            bottom,
                            from + 1,
                            to + 1
                        ));
                    }
                }
            }
        }
        // 2. Any legal tableau move.
        for from in 0..10 {
            let run = self.movable_run_len(from);
            if run == 0 {
                continue;
            }
            let len = self.tableau[from].len();
            let bottom = self.tableau[from][len - 1];
            if let Some(to) =
                (0..10).find(|&to| to != from && self.can_place_on_tableau(bottom, to))
            {
                return Some(format!(
                    "Move {} (tableau {}) onto tableau {}.",
                    bottom,
                    from + 1,
                    to + 1
                ));
            }
        }
        // 3. Deal, or explain why dealing is blocked.
        if !self.stock.is_empty() {
            if self.tableau.iter().any(|p| p.is_empty()) {
                return Some("Fill every empty column before dealing from the stock.".to_string());
            }
            return Some("Deal one card to each pile from the stock.".to_string());
        }
        None
    }
}

/// Build the 104-card Spider deck for a suit mode (face down).
fn build_deck(suits: SpiderSuits) -> Vec<Card> {
    let in_play = suits.suits_in_play();
    let copies = suits.copies_per_suit();
    let mut deck = Vec::with_capacity(104);
    for _ in 0..copies {
        for &suit in &in_play {
            for v in 1..=13 {
                deck.push(Card::new(suit, Rank(v)));
            }
        }
    }
    debug_assert_eq!(deck.len(), 104);
    deck
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deal_layout_54_plus_50() {
        for suits in [SpiderSuits::One, SpiderSuits::Two, SpiderSuits::Four] {
            let g = SpiderGame::new(suits);
            let tableau_total: usize = g.tableau.iter().map(|p| p.len()).sum();
            assert_eq!(tableau_total, 54, "{suits:?} tableau");
            assert_eq!(g.stock.len(), 50, "{suits:?} stock");
            assert!(g.tableau.iter().all(|p| p.last().unwrap().face_up));
            for (i, p) in g.tableau.iter().enumerate() {
                assert_eq!(p.len(), if i < 4 { 6 } else { 5 });
            }
        }
    }

    #[test]
    fn deck_composition_per_mode() {
        let one = build_deck(SpiderSuits::One);
        assert_eq!(one.len(), 104);
        assert!(one.iter().all(|c| c.suit == Suit::Spades));
        let two = build_deck(SpiderSuits::Two);
        assert!(two
            .iter()
            .all(|c| matches!(c.suit, Suit::Spades | Suit::Hearts)));
        let four = build_deck(SpiderSuits::Four);
        for s in Suit::all() {
            assert_eq!(four.iter().filter(|c| c.suit == s).count(), 26, "{s:?}");
        }
    }

    #[test]
    fn run_detection_stops_at_suit_break() {
        let mut g = SpiderGame::new(SpiderSuits::Two);
        for p in &mut g.tableau {
            p.clear();
        }
        g.stock.clear();
        let up = |s: Suit, v: u8| Card::new_face_up(s, Rank(v));
        g.tableau[0].push(up(Suit::Spades, 9));
        g.tableau[0].push(up(Suit::Spades, 8));
        g.tableau[0].push(up(Suit::Hearts, 7)); // suit break
        assert_eq!(g.movable_run_len(0), 1);
        // Any-rank-down placement still allowed for the 7.
        g.tableau[1].push(up(Suit::Clubs, 8));
        assert!(g.can_place_on_tableau(g.tableau_top(0).unwrap(), 1));
    }

    #[test]
    fn complete_sequence_banks_and_flips() {
        let mut g = SpiderGame::new(SpiderSuits::One);
        for p in &mut g.tableau {
            p.clear();
        }
        g.stock.clear();
        g.tableau[0].push(Card::new(Suit::Spades, Rank(5))); // stays face-down
        for v in (1..=13).rev() {
            g.tableau[0].push(Card::new_face_up(Suit::Spades, Rank(v)));
        }
        assert!(g.remove_completed_at(0));
        assert_eq!(g.completed_sequences, 1);
        assert_eq!(g.tableau[0].len(), 1);
        assert!(g.tableau[0][0].face_up); // exposed card flipped
    }

    #[test]
    fn four_onto_five_any_suit_is_legal() {
        // Spider builds down by rank regardless of suit; empty columns
        // take any card.
        let mut g = SpiderGame::new(SpiderSuits::Two);
        for p in &mut g.tableau {
            p.clear();
        }
        g.stock.clear();
        let up = |s: Suit, v: u8| Card::new_face_up(s, Rank(v));
        g.tableau[0].push(up(Suit::Hearts, 5));
        assert!(g.can_place_on_tableau(up(Suit::Clubs, 4), 0));
        g.tableau[1].push(up(Suit::Clubs, 4));
        assert!(g.move_tableau_to_tableau(1, 1, 0));
        assert_eq!(g.tableau[0].len(), 2);
        // Rank gap and ascending order stay illegal; empties take any.
        g.tableau[3].push(up(Suit::Hearts, 5));
        assert!(!g.can_place_on_tableau(up(Suit::Clubs, 3), 3));
        g.tableau[4].push(up(Suit::Clubs, 4));
        assert!(!g.can_place_on_tableau(up(Suit::Hearts, 6), 4));
        assert!(g.can_place_on_tableau(up(Suit::Clubs, 4), 5));
    }

    #[test]
    fn deal_refused_with_empty_column() {
        let mut g = SpiderGame::new(SpiderSuits::One);
        g.tableau[9].clear();
        assert!(!g.can_deal());
        assert!(!g.deal_from_stock());
        assert_eq!(g.stock_deals_remaining(), 5);
    }
}
