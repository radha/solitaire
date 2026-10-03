//! Spider solitaire state and rules.
//!
//! Layout: 10 tableau piles, a stock that deals one card to every pile,
//! and a count of completed sequences.
//! - Tableau builds down by rank regardless of suit; any card (run) may
//!   occupy an empty column.
//! - Only same-suit descending runs may move together.
//! - A complete K -> A same-suit run (13 cards) leaves the table.
//! - The stock deals 10 cards at a time and refuses while any column is
//!   empty (classic rule): 5 deals from a 104-card deck.
//! - Difficulty = suits in play: Easy = 1, Normal = 2, Hard = 4.
//! - Scoring (Windows style): start at 500, -1 per move or deal, +100 per
//!   completed sequence.

use std::fmt;

use crate::cards::{Card, DealRng, Rank, Suit};
use crate::rules::{Difficulty, MoveError, Solitaire, cards};

pub const TABLEAU_PILES: usize = 10;
pub const SEQUENCES_TO_WIN: u8 = 8;
pub const SCORE_START: i32 = 500;
pub const SCORE_PER_MOVE: i32 = -1;
pub const SCORE_SEQUENCE_COMPLETE: i32 = 100;

/// Suits in play (difficulty selector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiderSuits {
    One,
    Two,
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

    pub fn suits_in_play(self) -> &'static [Suit] {
        match self {
            SpiderSuits::One => &[Suit::Spades],
            SpiderSuits::Two => &[Suit::Spades, Suit::Hearts],
            SpiderSuits::Four => &Suit::ALL,
        }
    }
}

impl fmt::Display for SpiderSuits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SpiderSuits::One => "1 suit",
            SpiderSuits::Two => "2 suits",
            SpiderSuits::Four => "4 suits",
        })
    }
}

/// Build the shuffled 104-card deck (face down) for a suit mode and deal.
fn shuffled_deck(suits: SpiderSuits, deal: u32) -> Vec<Card> {
    let in_play = suits.suits_in_play();
    let copies = 8 / in_play.len();
    let mut deck: Vec<Card> = (0..copies)
        .flat_map(|_| in_play.iter())
        .flat_map(|&suit| Rank::all().map(move |rank| Card::new(suit, rank)))
        .collect();
    debug_assert_eq!(deck.len(), 104);
    DealRng::new(deal).shuffle(&mut deck);
    deck
}

/// A move suggested by [`SpiderGame::best_move`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Move {
        from: usize,
        to: usize,
        count: usize,
    },
    Deal,
    /// Advice only: the stock can't be dealt until every column has a card.
    FillEmptyColumns,
    /// Advice only: an empty column is available for rearranging.
    UseEmptyColumn,
}

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Hint::Move { from, to, count } => write!(
                f,
                "Move {} from tableau {} onto tableau {}.",
                cards(count),
                from + 1,
                to + 1
            ),
            Hint::Deal => f.write_str("Deal one card to each pile from the stock."),
            Hint::FillEmptyColumns => {
                f.write_str("Fill every empty column (any card goes) before dealing.")
            }
            Hint::UseEmptyColumn => f.write_str("Use the empty column to rearrange runs."),
        }
    }
}

/// Spider game state.
#[derive(Debug, Clone)]
pub struct SpiderGame {
    pub suits: SpiderSuits,
    pub tableau: [Vec<Card>; TABLEAU_PILES],
    pub stock: Vec<Card>,
    pub completed_sequences: u8,
    score: i32,
    moves: u32,
    deal: u32,
}

impl SpiderGame {
    pub fn new(suits: SpiderSuits, deal: u32) -> Self {
        let mut game = Self {
            suits,
            tableau: Default::default(),
            stock: Vec::new(),
            completed_sequences: 0,
            score: SCORE_START,
            moves: 0,
            deal,
        };
        game.restart();
        game
    }

    /// Re-deal the same deal number: 54 cards to the tableau (piles 0-3 get
    /// 6, piles 4-9 get 5), tops face-up; the other 50 form the stock.
    pub fn restart(&mut self) {
        let mut deck = shuffled_deck(self.suits, self.deal);
        for (i, pile) in self.tableau.iter_mut().enumerate() {
            let n = if i < 4 { 6 } else { 5 };
            pile.clear();
            pile.extend(deck.drain(deck.len() - n..));
            if let Some(top) = pile.last_mut() {
                top.face_up = true;
            }
        }
        self.stock = deck;
        self.completed_sequences = 0;
        self.score = SCORE_START;
        self.moves = 0;
    }

    // ---- accessors ----

    pub fn stock_deals_remaining(&self) -> usize {
        self.stock.len() / TABLEAU_PILES
    }

    // ---- rules ----

    /// Tableau rule: empty takes anything; otherwise one rank lower, any suit.
    pub fn check_tableau(&self, card: Card, to: usize) -> Result<(), MoveError> {
        let pile = self.tableau.get(to).ok_or(MoveError::NotAllowed)?;
        match pile.last() {
            None => Ok(()),
            Some(top) if top.face_up && top.rank.value() == card.rank.value() + 1 => Ok(()),
            Some(_) => Err(MoveError::WrongRankAnySuit),
        }
    }

    /// Length of the same-suit descending face-up run ending at the top:
    /// the most cards that may move together from `pile`.
    pub fn movable_run_len(&self, pile: usize) -> usize {
        let Some(p) = self.tableau.get(pile) else {
            return 0;
        };
        if !p.last().is_some_and(|c| c.face_up) {
            return 0;
        }
        1 + p
            .windows(2)
            .rev()
            .take_while(|w| {
                let (below, above) = (w[0], w[1]);
                below.face_up
                    && below.suit == above.suit
                    && below.rank.value() == above.rank.value() + 1
            })
            .count()
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
        if count == 0 || count > self.movable_run_len(from) {
            return Err(MoveError::NotARun);
        }
        let pile = &self.tableau[from];
        self.check_tableau(pile[pile.len() - count], to)
    }

    fn flip_top(&mut self, pile: usize) {
        if let Some(top) = self.tableau[pile].last_mut() {
            top.face_up = true;
        }
    }

    fn count_move(&mut self) {
        self.moves += 1;
        self.score = (self.score + SCORE_PER_MOVE).max(0);
    }

    /// Remove a complete K..A same-suit run ending at the pile top.
    fn remove_completed_at(&mut self, pile: usize) -> bool {
        if self.movable_run_len(pile) < 13 {
            return false;
        }
        let p = &mut self.tableau[pile];
        let start = p.len() - 13;
        if !p[start].rank.is_king() {
            return false;
        }
        p.truncate(start);
        self.completed_sequences += 1;
        self.score += SCORE_SEQUENCE_COMPLETE;
        self.flip_top(pile);
        true
    }

    /// Move exactly the top `count` cards between tableau piles.
    pub fn move_tableau_to_tableau(&mut self, from: usize, to: usize, count: usize) -> bool {
        if self.check_tableau_move(from, to, count).is_err() {
            return false;
        }
        let start = self.tableau[from].len() - count;
        let moving: Vec<Card> = self.tableau[from].drain(start..).collect();
        self.tableau[to].extend(moving);
        self.flip_top(from);
        self.count_move();
        self.remove_completed_at(to);
        true
    }

    /// Whether the stock may be dealt now (no empty column, per the rules).
    pub fn can_deal(&self) -> bool {
        self.stock.len() >= TABLEAU_PILES && self.tableau.iter().all(|p| !p.is_empty())
    }

    /// Deal one face-up card to each tableau pile.
    pub fn deal_from_stock(&mut self) -> bool {
        if !self.can_deal() {
            return false;
        }
        for pile in 0..TABLEAU_PILES {
            if let Some(mut c) = self.stock.pop() {
                c.face_up = true;
                self.tableau[pile].push(c);
            }
        }
        self.count_move();
        for pile in 0..TABLEAU_PILES {
            self.remove_completed_at(pile);
        }
        true
    }

    /// Next useful move. Every suggested move either joins a run to its
    /// same-suit successor, builds onto a card where the run wasn't built
    /// before, or uncovers a face-down card, so following hints never loops.
    pub fn best_move(&self) -> Option<Hint> {
        if self.is_won() {
            return None;
        }
        let mut builds = None;
        let mut empty_target = None;
        for from in 0..TABLEAU_PILES {
            let count = self.movable_run_len(from);
            if count == 0 {
                continue;
            }
            let pile = &self.tableau[from];
            let bottom = pile[pile.len() - count];
            let beneath = pile.len().checked_sub(count + 1).map(|i| pile[i]);
            let built_on_beneath =
                beneath.is_some_and(|b| b.face_up && b.rank.value() == bottom.rank.value() + 1);
            for to in (0..TABLEAU_PILES).filter(|&to| to != from) {
                if self.check_tableau_move(from, to, count).is_err() {
                    continue;
                }
                match self.tableau[to].last() {
                    Some(top) if top.suit == bottom.suit => {
                        return Some(Hint::Move { from, to, count });
                    }
                    Some(_) if !built_on_beneath && builds.is_none() => {
                        builds = Some(Hint::Move { from, to, count });
                    }
                    None if beneath.is_some_and(|b| !b.face_up) && empty_target.is_none() => {
                        empty_target = Some(Hint::Move { from, to, count });
                    }
                    _ => {}
                }
            }
        }
        builds.or(empty_target).or_else(|| {
            let has_empty = self.tableau.iter().any(Vec::is_empty);
            if self.can_deal() {
                Some(Hint::Deal)
            } else if has_empty && !self.stock.is_empty() {
                Some(Hint::FillEmptyColumns)
            } else if has_empty {
                Some(Hint::UseEmptyColumn)
            } else {
                None
            }
        })
    }
}

impl Solitaire for SpiderGame {
    fn score(&self) -> i32 {
        self.score
    }

    fn move_count(&self) -> u32 {
        self.moves
    }

    fn is_won(&self) -> bool {
        self.completed_sequences >= SEQUENCES_TO_WIN
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

    fn up(s: Suit, v: u8) -> Card {
        Card::up(s, v)
    }

    fn empty_game(suits: SpiderSuits) -> SpiderGame {
        let mut g = SpiderGame::new(suits, 1);
        g.tableau.iter_mut().for_each(Vec::clear);
        g.stock.clear();
        g
    }

    #[test]
    fn deal_layout_54_plus_50() {
        for suits in [SpiderSuits::One, SpiderSuits::Two, SpiderSuits::Four] {
            let g = SpiderGame::new(suits, 3);
            assert_eq!(
                g.tableau.iter().map(Vec::len).sum::<usize>(),
                54,
                "{suits:?}"
            );
            assert_eq!(g.stock.len(), 50, "{suits:?} stock");
            for (i, p) in g.tableau.iter().enumerate() {
                assert_eq!(p.len(), if i < 4 { 6 } else { 5 });
                assert!(p.last().unwrap().face_up);
                assert!(p[..p.len() - 1].iter().all(|c| !c.face_up));
            }
            assert_eq!(g.score(), SCORE_START);
        }
    }

    #[test]
    fn deck_composition_per_mode() {
        let one = shuffled_deck(SpiderSuits::One, 1);
        assert_eq!(one.len(), 104);
        assert!(one.iter().all(|c| c.suit == Suit::Spades));
        let two = shuffled_deck(SpiderSuits::Two, 1);
        assert!(
            two.iter()
                .all(|c| matches!(c.suit, Suit::Spades | Suit::Hearts))
        );
        let four = shuffled_deck(SpiderSuits::Four, 1);
        for s in Suit::ALL {
            assert_eq!(four.iter().filter(|c| c.suit == s).count(), 26, "{s:?}");
        }
    }

    #[test]
    fn run_detection_stops_at_suit_break() {
        let mut g = empty_game(SpiderSuits::Two);
        g.tableau[0].extend([
            up(Suit::Spades, 9),
            up(Suit::Spades, 8),
            up(Suit::Hearts, 7),
        ]);
        assert_eq!(g.movable_run_len(0), 1);
        g.tableau[1].extend([up(Suit::Spades, 9), up(Suit::Spades, 8)]);
        assert_eq!(g.movable_run_len(1), 2);
        g.tableau[2].push(up(Suit::Clubs, 8));
        assert!(g.check_tableau(up(Suit::Hearts, 7), 2).is_ok());
    }

    #[test]
    fn complete_sequence_banks_and_flips() {
        let mut g = empty_game(SpiderSuits::One);
        g.tableau[0].push(Card::new(Suit::Spades, Rank::new(5)));
        g.tableau[0].extend((2..=13).rev().map(|v| up(Suit::Spades, v)));
        g.tableau[1].push(up(Suit::Spades, 1));
        assert!(g.move_tableau_to_tableau(1, 0, 1));
        assert_eq!(g.completed_sequences, 1);
        assert_eq!(g.tableau[0].len(), 1);
        assert!(g.tableau[0][0].face_up, "exposed card flipped");
        assert_eq!(
            g.score(),
            SCORE_START + SCORE_PER_MOVE + SCORE_SEQUENCE_COMPLETE
        );
    }

    #[test]
    fn moves_cost_a_point_and_are_exact() {
        let mut g = empty_game(SpiderSuits::Two);
        g.tableau[0].push(up(Suit::Hearts, 5));
        g.tableau[1].extend([up(Suit::Clubs, 5), up(Suit::Clubs, 4)]);
        assert!(g.move_tableau_to_tableau(1, 0, 1), "4♣ onto 5♥ (any suit)");
        assert_eq!(g.score(), SCORE_START + SCORE_PER_MOVE);
        assert!(g.move_tableau_to_tableau(0, 1, 1));
        assert_eq!(g.score(), SCORE_START + 2 * SCORE_PER_MOVE, "no farming");
        assert_eq!(g.check_tableau_move(1, 0, 3), Err(MoveError::NotARun));
        assert_eq!(
            g.check_tableau(up(Suit::Hearts, 6), 1),
            Err(MoveError::WrongRankAnySuit)
        );
    }

    #[test]
    fn deal_refused_with_empty_column() {
        let mut g = SpiderGame::new(SpiderSuits::One, 1);
        g.tableau[9].clear();
        assert!(!g.can_deal());
        assert!(!g.deal_from_stock());
        assert_eq!(g.stock_deals_remaining(), 5);
    }

    #[test]
    fn hints_skip_lateral_moves() {
        // 7♠ sits on 8♥ (built). Moving it onto 8♦ is lateral: no hint.
        let mut g = empty_game(SpiderSuits::Four);
        for p in &mut g.tableau {
            p.push(up(Suit::Clubs, 13)); // Kings never move onto anything
        }
        g.tableau[0] = vec![up(Suit::Hearts, 8), up(Suit::Spades, 7)];
        g.tableau[1] = vec![up(Suit::Diamonds, 8)];
        assert_eq!(g.best_move(), None);
        // A same-suit home is always worth it.
        g.tableau[1] = vec![up(Suit::Spades, 8)];
        assert_eq!(
            g.best_move(),
            Some(Hint::Move {
                from: 0,
                to: 1,
                count: 1
            })
        );
    }

    #[test]
    fn following_hints_terminates() {
        for deal in 0..20 {
            let mut g = SpiderGame::new(SpiderSuits::Two, deal);
            let mut steps = 0;
            while let Some(h) = g.best_move() {
                let applied = match h {
                    Hint::Move { from, to, count } => g.move_tableau_to_tableau(from, to, count),
                    Hint::Deal => g.deal_from_stock(),
                    Hint::FillEmptyColumns | Hint::UseEmptyColumn => break,
                };
                assert!(applied, "deal {deal}: {h} not applicable");
                steps += 1;
                assert!(steps < 2_000, "deal {deal}: hints looped");
            }
        }
    }
}
