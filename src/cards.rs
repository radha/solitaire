//! Core card types shared by all solitaire variants, plus the seeded
//! shuffle that turns a deal number into a reproducible deck.

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
    pub const ALL: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

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

/// Card rank: Ace = 1 .. King = 13. The field is private so every `Rank`
/// in the program is in range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank(u8);

impl Rank {
    pub const ACE: Rank = Rank(1);
    pub const KING: Rank = Rank(13);

    /// Build a rank from its value (tests; production iterates
    /// [`Rank::all`]). Panics outside 1..=13.
    #[cfg(test)]
    pub const fn new(value: u8) -> Rank {
        assert!(value >= 1 && value <= 13, "rank out of range");
        Rank(value)
    }

    /// Every rank, Ace to King.
    pub fn all() -> impl Iterator<Item = Rank> {
        (1..=13).map(Rank)
    }

    /// Numeric value 1..=13.
    pub const fn value(self) -> u8 {
        self.0
    }

    pub fn is_ace(self) -> bool {
        self == Rank::ACE
    }

    pub fn is_king(self) -> bool {
        self == Rank::KING
    }

    pub fn label(self) -> &'static str {
        const LABELS: [&str; 13] = [
            "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K",
        ];
        LABELS[usize::from(self.0 - 1)]
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
    /// A face-down card.
    pub const fn new(suit: Suit, rank: Rank) -> Self {
        Self {
            suit,
            rank,
            face_up: false,
        }
    }

    /// Test-only face-up constructor taking a raw rank value.
    #[cfg(test)]
    pub const fn up(suit: Suit, value: u8) -> Self {
        Self {
            suit,
            rank: Rank::new(value),
            face_up: true,
        }
    }

    pub fn is_red(self) -> bool {
        self.suit.is_red()
    }

    pub fn color(self) -> CardColor {
        self.suit.color()
    }
}

/// Face-up cards render like `A♠` / `10♦`; face-down cards render as `??`.
impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.face_up {
            write!(f, "{}{}", self.rank.label(), self.suit.symbol())
        } else {
            f.write_str("??")
        }
    }
}

/// Build a standard 52-card deck (face down).
pub fn full_deck() -> Vec<Card> {
    Suit::ALL
        .iter()
        .flat_map(|&suit| Rank::all().map(move |rank| Card::new(suit, rank)))
        .collect()
}

/// Shuffle a 52-card deck for deal number `deal`.
pub fn shuffled_deck(deal: u32) -> Vec<Card> {
    let mut deck = full_deck();
    DealRng::new(deal).shuffle(&mut deck);
    deck
}

/// A fresh random deal number.
pub fn random_deal() -> u32 {
    rand::random()
}

/// Small deterministic PRNG (SplitMix64). Implemented here rather than
/// taken from `rand` so a deal number reproduces the same shuffle forever,
/// independent of the `rand` crate's algorithm choices across versions.
pub struct DealRng(u64);

impl DealRng {
    pub fn new(deal: u32) -> Self {
        Self(u64::from(deal))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform index in `0..n` (multiply-shift; bias is negligible for
    /// deck-sized `n`).
    fn below(&mut self, n: usize) -> usize {
        let wide = u128::from(self.next_u64()) * n as u128;
        usize::try_from(wide >> 64).expect("index below n fits usize")
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

/// A movable stack must be face-up and strictly descending with alternating
/// colors. Shared by Klondike and FreeCell tableau-run validation.
pub fn is_valid_descending_alternating_run(cards: &[Card]) -> bool {
    !cards.is_empty()
        && cards.iter().all(|c| c.face_up)
        && cards
            .windows(2)
            .all(|w| w[0].color() != w[1].color() && w[0].rank.value() == w[1].rank.value() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
    fn card_display() {
        assert_eq!(Card::up(Suit::Spades, 1).to_string(), "A♠");
        assert_eq!(Card::up(Suit::Diamonds, 10).to_string(), "10♦");
        assert_eq!(Card::up(Suit::Hearts, 13).to_string(), "K♥");
        assert_eq!(Card::up(Suit::Clubs, 12).to_string(), "Q♣");
        assert_eq!(Card::new(Suit::Spades, Rank::ACE).to_string(), "??");
    }

    #[test]
    fn full_deck_has_52_unique_cards() {
        let deck = full_deck();
        assert_eq!(deck.len(), 52);
        let unique: HashSet<_> = deck.iter().map(|c| (c.suit, c.rank)).collect();
        assert_eq!(unique.len(), 52);
    }

    #[test]
    fn rank_helpers() {
        assert!(Rank::ACE.is_ace());
        assert!(Rank::KING.is_king());
        assert_eq!(Rank::new(7).value(), 7);
        assert_eq!(Rank::all().count(), 13);
        assert_eq!(Rank::new(10).label(), "10");
    }

    #[test]
    #[should_panic(expected = "rank out of range")]
    fn rank_rejects_out_of_range() {
        let _ = Rank::new(14);
    }

    #[test]
    fn deal_numbers_are_reproducible() {
        assert_eq!(shuffled_deck(42), shuffled_deck(42));
        assert_ne!(shuffled_deck(42), shuffled_deck(43));
        let unique: HashSet<_> = shuffled_deck(7).iter().map(|c| (c.suit, c.rank)).collect();
        assert_eq!(unique.len(), 52);
    }
}
