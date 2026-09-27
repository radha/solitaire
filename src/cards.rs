//! Core card types shared by all solitaire variants.

use rand::seq::SliceRandom;
use rand::thread_rng;
use std::fmt;

/// Card suit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Suit {
    Spades,
    Hearts,
    Diamonds,
    Clubs,
}

/// Red/black card color (drives tableau alternating-color rules and UI styling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardColor {
    Red,
    Black,
}

impl Suit {
    pub fn all() -> [Suit; 4] {
        [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs]
    }

    pub fn is_red(self) -> bool {
        matches!(self, Suit::Hearts | Suit::Diamonds)
    }

    pub fn color(self) -> CardColor {
        if self.is_red() {
            CardColor::Red
        } else {
            CardColor::Black
        }
    }

    /// Unicode suit symbol: ♠ ♥ ♦ ♣.
    pub fn symbol(self) -> char {
        match self {
            Suit::Spades => '♠',
            Suit::Hearts => '♥',
            Suit::Diamonds => '♦',
            Suit::Clubs => '♣',
        }
    }
}

/// Card rank: Ace = 1 .. King = 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank(pub u8);

impl Rank {
    pub const ACE: Rank = Rank(1);
    pub const JACK: Rank = Rank(11);
    pub const QUEEN: Rank = Rank(12);
    pub const KING: Rank = Rank(13);

    pub fn new(v: u8) -> Option<Rank> {
        if (1..=13).contains(&v) {
            Some(Rank(v))
        } else {
            None
        }
    }

    /// Numeric value 1..=13.
    pub fn value(self) -> u8 {
        self.0
    }

    pub fn is_ace(self) -> bool {
        self.0 == 1
    }

    pub fn is_king(self) -> bool {
        self.0 == 13
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            1 => "A",
            11 => "J",
            12 => "Q",
            13 => "K",
            2 => "2",
            3 => "3",
            4 => "4",
            5 => "5",
            6 => "6",
            7 => "7",
            8 => "8",
            9 => "9",
            10 => "10",
            _ => "?",
        }
    }
}

/// A single playing card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Card {
    pub suit: Suit,
    pub rank: Rank,
    pub face_up: bool,
}

impl Card {
    pub fn new(suit: Suit, rank: Rank) -> Self {
        Self {
            suit,
            rank,
            face_up: false,
        }
    }

    pub fn new_face_up(suit: Suit, rank: Rank) -> Self {
        Self {
            suit,
            rank,
            face_up: true,
        }
    }

    pub fn is_red(&self) -> bool {
        self.suit.is_red()
    }

    pub fn color(&self) -> CardColor {
        self.suit.color()
    }

    pub fn is_ace(&self) -> bool {
        self.rank.is_ace()
    }

    pub fn is_king(&self) -> bool {
        self.rank.is_king()
    }

    /// Face-up render like `A♠`, `10♦`. Face-down cards render as the back.
    pub fn render(&self) -> String {
        if self.face_up {
            format!("{}{}", self.rank.label(), self.suit.symbol())
        } else {
            Self::back_str().to_string()
        }
    }

    /// Card-back render for face-down / stock cards.
    pub fn back_str() -> &'static str {
        "??"
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// Build a standard 52-card deck (face down).
pub fn full_deck() -> Vec<Card> {
    let mut deck = Vec::with_capacity(52);
    for &suit in &Suit::all() {
        for v in 1..=13 {
            deck.push(Card::new(suit, Rank(v)));
        }
    }
    deck
}

/// Shuffled 52-card deck.
pub fn shuffled_deck() -> Vec<Card> {
    let mut deck = full_deck();
    deck.shuffle(&mut thread_rng());
    deck
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suit_symbols_and_colors() {
        assert_eq!(Suit::Spades.symbol(), '♠');
        assert_eq!(Suit::Hearts.symbol(), '♥');
        assert_eq!(Suit::Diamonds.symbol(), '♦');
        assert_eq!(Suit::Clubs.symbol(), '♣');
        assert_eq!(Suit::Hearts.color(), CardColor::Red);
        assert_eq!(Suit::Diamonds.color(), CardColor::Red);
        assert_eq!(Suit::Spades.color(), CardColor::Black);
        assert_eq!(Suit::Clubs.color(), CardColor::Black);
    }

    #[test]
    fn card_string_render() {
        let ace = Card::new_face_up(Suit::Spades, Rank::ACE);
        assert_eq!(ace.render(), "A♠");
        assert_eq!(ace.to_string(), "A♠");
        let ten = Card::new_face_up(Suit::Diamonds, Rank(10));
        assert_eq!(ten.render(), "10♦");
        let king = Card::new_face_up(Suit::Hearts, Rank::KING);
        assert_eq!(king.render(), "K♥");
        let queen = Card::new_face_up(Suit::Clubs, Rank::QUEEN);
        assert_eq!(queen.render(), "Q♣");
    }

    #[test]
    fn card_back_render() {
        let down = Card::new(Suit::Spades, Rank::ACE);
        assert_eq!(down.render(), Card::back_str());
        assert_eq!(down.to_string(), "??");
    }

    #[test]
    fn full_deck_has_52_unique_cards() {
        let deck = full_deck();
        assert_eq!(deck.len(), 52);
        let mut seen = std::collections::HashSet::new();
        for c in &deck {
            assert!(seen.insert((c.suit, c.rank)), "duplicate card {c}");
        }
    }

    #[test]
    fn rank_helpers() {
        assert!(Rank::ACE.is_ace());
        assert!(Rank::KING.is_king());
        assert_eq!(Rank::new(0), None);
        assert_eq!(Rank::new(14), None);
        assert_eq!(Rank::new(7).unwrap().value(), 7);
    }
}
