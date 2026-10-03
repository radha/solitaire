//! Rules shared across variants: difficulty, the common game interface,
//! move errors, and the foundation / alternating-color building rules.

use std::fmt;

use crate::cards::{Card, Suit};

/// Difficulty level. Each game maps it to its own knob (Klondike draw count
/// and redeal limit, FreeCell cell count, Spider suit count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn harder(self) -> Self {
        match self {
            Difficulty::Easy => Difficulty::Normal,
            Difficulty::Normal | Difficulty::Hard => Difficulty::Hard,
        }
    }

    pub fn easier(self) -> Self {
        match self {
            Difficulty::Hard => Difficulty::Normal,
            Difficulty::Normal | Difficulty::Easy => Difficulty::Easy,
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        })
    }
}

/// What every variant exposes to the app shell.
pub trait Solitaire {
    fn score(&self) -> i32;
    fn move_count(&self) -> u32;
    fn is_won(&self) -> bool;
    /// Deal number that reproduces this game's starting layout.
    fn deal(&self) -> u32;
    /// A suggested next move. Hints only suggest moves that make progress,
    /// so following them never loops; `None` means no useful move is left.
    fn hint(&self) -> Option<String>;

    /// No useful move remains and the game isn't won.
    fn is_stuck(&self) -> bool {
        !self.is_won() && self.hint().is_none()
    }
}

/// Why a move was refused. `Display` gives the player-facing explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveError {
    FoundationNeedsAce,
    FoundationWrongSuit,
    FoundationWrongRank,
    EmptyColumnNeedsKing,
    SameColor,
    WrongRank,
    WrongRankAnySuit,
    NotARun,
    TooMany { count: usize, max: usize },
    CellOccupied,
    NotAllowed,
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Illegal move: ")?;
        match self {
            MoveError::FoundationNeedsAce => f.write_str("foundations start with an Ace."),
            MoveError::FoundationWrongSuit => {
                f.write_str("foundations only take the same suit, one rank up.")
            }
            MoveError::FoundationWrongRank => f.write_str("foundations need the next rank up."),
            MoveError::EmptyColumnNeedsKing => {
                f.write_str("only a King can start an empty column.")
            }
            MoveError::SameColor => f.write_str("tableau piles need alternating colors."),
            MoveError::WrongRank => {
                f.write_str("tableau piles need one rank lower than the destination card.")
            }
            MoveError::WrongRankAnySuit => f.write_str(
                "tableau piles need one rank lower than the destination card (any suit).",
            ),
            MoveError::NotARun => f.write_str("those cards don't form a movable run."),
            MoveError::TooMany { count, max } => write!(
                f,
                "not enough free cells/empty columns to move {count} cards at once (max {max} here)."
            ),
            MoveError::CellOccupied => f.write_str("that free cell is already in use."),
            MoveError::NotAllowed => f.write_str("those cards cannot go there."),
        }
    }
}

/// Foundation rule shared by Klondike and FreeCell: empty takes an Ace,
/// otherwise same suit, one rank up.
pub fn check_foundation(pile: &[Card], card: Card) -> Result<(), MoveError> {
    match pile.last() {
        None if card.rank.is_ace() => Ok(()),
        None => Err(MoveError::FoundationNeedsAce),
        Some(top) if top.suit != card.suit => Err(MoveError::FoundationWrongSuit),
        Some(top) if card.rank.value() != top.rank.value() + 1 => {
            Err(MoveError::FoundationWrongRank)
        }
        Some(_) => Ok(()),
    }
}

/// Index of the foundation pile that accepts `card`, if any.
pub fn foundation_for(foundations: &[Vec<Card>], card: Card) -> Option<usize> {
    foundations
        .iter()
        .position(|pile| check_foundation(pile, card).is_ok())
}

/// Alternating-color, one-rank-down building rule (Klondike, FreeCell).
pub fn check_alternating(top: Card, card: Card) -> Result<(), MoveError> {
    if !top.face_up {
        Err(MoveError::NotAllowed)
    } else if top.color() == card.color() {
        Err(MoveError::SameColor)
    } else if top.rank.value() != card.rank.value() + 1 {
        Err(MoveError::WrongRank)
    } else {
        Ok(())
    }
}

/// Height (top rank) of `suit`'s foundation; 0 when it hasn't started.
fn suit_height(foundations: &[Vec<Card>], suit: Suit) -> u8 {
    foundations
        .iter()
        .filter_map(|pile| pile.last())
        .find(|top| top.suit == suit)
        .map_or(0, |top| top.rank.value())
}

/// Classic safe auto-play rule: a card can go up without risk when no
/// tableau card could still need it as a building target, i.e. its rank is
/// 2 or less, or *both* opposite-color suits are already up to rank - 1.
/// A suit whose foundation hasn't started counts as height 0.
pub fn is_safe_for_foundation(foundations: &[Vec<Card>], card: Card) -> bool {
    let rank = card.rank.value();
    rank <= 2
        || Suit::ALL
            .iter()
            .filter(|s| s.is_red() != card.is_red())
            .all(|&s| suit_height(foundations, s) >= rank - 1)
}

/// "card" / "cards" for counts in status text.
pub fn cards(n: usize) -> String {
    if n == 1 {
        "1 card".to_string()
    } else {
        format!("{n} cards")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundation_rule() {
        let ace = Card::up(Suit::Spades, 1);
        assert_eq!(check_foundation(&[], ace), Ok(()));
        assert_eq!(
            check_foundation(&[], Card::up(Suit::Spades, 2)),
            Err(MoveError::FoundationNeedsAce)
        );
        assert_eq!(
            check_foundation(&[ace], Card::up(Suit::Hearts, 2)),
            Err(MoveError::FoundationWrongSuit)
        );
        assert_eq!(
            check_foundation(&[ace], Card::up(Suit::Spades, 3)),
            Err(MoveError::FoundationWrongRank)
        );
        assert_eq!(check_foundation(&[ace], Card::up(Suit::Spades, 2)), Ok(()));
    }

    #[test]
    fn safe_rule_treats_unstarted_suits_as_zero() {
        // Hearts up to 2 but diamonds not started: a black 3 could still be
        // needed to hold 2♦, so it is not safe.
        let foundations = vec![
            vec![Card::up(Suit::Spades, 1), Card::up(Suit::Spades, 2)],
            vec![Card::up(Suit::Hearts, 1), Card::up(Suit::Hearts, 2)],
            vec![],
            vec![],
        ];
        assert!(!is_safe_for_foundation(
            &foundations,
            Card::up(Suit::Spades, 3)
        ));
        assert!(is_safe_for_foundation(
            &foundations,
            Card::up(Suit::Hearts, 2)
        ));
        let mut both = foundations.clone();
        both[2] = vec![Card::up(Suit::Diamonds, 1), Card::up(Suit::Diamonds, 2)];
        assert!(is_safe_for_foundation(&both, Card::up(Suit::Spades, 3)));
    }

    #[test]
    fn plural_helper() {
        assert_eq!(cards(1), "1 card");
        assert_eq!(cards(3), "3 cards");
    }
}
