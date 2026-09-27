//! Full Klondike solitaire state and rules.
//!
//! Layout: 7 tableau piles, 4 foundations, stock + waste.
//! Draw modes: draw-1 / draw-3. Difficulty mapping:
//! - Easy: draw-1, unlimited redeals, hint system available.
//! - Normal: draw-3, unlimited redeals.
//! - Hard: draw-3, limited stock redeals (`max_passes`, default 3);
//!   refusing a redeal ends the game.

use crate::cards::{is_valid_descending_alternating_run, shuffled_deck, Card, Rank, Suit};

/// Draw mode for Klondike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawMode {
    /// Easy mode: draw 1.
    #[default]
    Draw1,
    /// Classic mode: draw 3.
    Draw3,
}

/// Difficulty level: controls redeal limits (and the recommended draw mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Difficulty {
    #[default]
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    /// Recommended draw mode per difficulty. Documents the mapping (and
    /// guards it with a test); draw mode is otherwise chosen independently
    /// via `--draw`, not derived from difficulty in production.
    #[cfg(test)]
    pub fn recommended_draw_mode(self) -> DrawMode {
        match self {
            Difficulty::Easy => DrawMode::Draw1,
            Difficulty::Normal | Difficulty::Hard => DrawMode::Draw3,
        }
    }

    /// Whether the stock may be recycled without limit.
    pub fn unlimited_redeals(self) -> bool {
        !matches!(self, Difficulty::Hard)
    }

    /// Default redeal allowance for Hard (number of stock recycles).
    pub fn default_max_passes() -> u32 {
        3
    }
}

// Scoring (Microsoft-style-ish, documented here):
pub const SCORE_WASTE_TO_TABLEAU: i32 = 5;
pub const SCORE_TO_FOUNDATION: i32 = 10;
pub const SCORE_TABLEAU_TO_TABLEAU: i32 = 3;
pub const SCORE_FLIP_TABLEAU: i32 = 5;
pub const SCORE_FOUNDATION_TO_TABLEAU: i32 = -15;
pub const SCORE_REDEAL_PENALTY: i32 = 20;
pub const SCORE_WIN_BONUS: i32 = 500;

/// Suggested move returned by [`KlondikeGame::hint`].
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
    FoundationToTableau {
        foundation: usize,
        tableau: usize,
    },
    Draw,
    Redeal,
    AutoFinish,
}

impl Hint {
    pub fn describe(self) -> String {
        match self {
            Hint::WasteToFoundation => "Move waste card to foundation.".to_string(),
            Hint::WasteToTableau(t) => format!("Move waste card to tableau pile {}.", t + 1),
            Hint::TableauToFoundation(t) => {
                format!("Move tableau pile {} top card to foundation.", t + 1)
            }
            Hint::TableauToTableau { from, to, count } => format!(
                "Move {count} card(s) from tableau pile {} to pile {}.",
                from + 1,
                to + 1
            ),
            Hint::FoundationToTableau {
                foundation,
                tableau,
            } => format!(
                "Move foundation pile {} top card back to tableau pile {}.",
                foundation + 1,
                tableau + 1
            ),
            Hint::Draw => "Draw from stock.".to_string(),
            Hint::Redeal => "Recycle waste back into stock.".to_string(),
            Hint::AutoFinish => "Auto-finish: everything can go to foundations.".to_string(),
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
    pub foundations: [Vec<Card>; 4],
    pub tableau: [Vec<Card>; 7],
    score: i32,
    moves: u32,
    redeals_used: u32,
    max_passes: u32,
    game_over: bool,
    winnable_deal: bool,
    win_bonus_awarded: bool,
}

impl Default for KlondikeGame {
    fn default() -> Self {
        Self::new(DrawMode::Draw1, Difficulty::Easy)
    }
}

impl KlondikeGame {
    pub fn new(draw_mode: DrawMode, difficulty: Difficulty) -> Self {
        let mut game = Self::empty(draw_mode, difficulty, false);
        game.reset();
        game
    }

    /// New game with a winnable-slanted deal. Test-only constructor;
    /// production reaches the same deal via `reset_winnable` on an
    /// existing game (see `App::new_game`).
    #[cfg(test)]
    pub fn new_winnable(draw_mode: DrawMode, difficulty: Difficulty) -> Self {
        let mut game = Self::empty(draw_mode, difficulty, true);
        game.reset_winnable();
        game
    }

    fn empty(draw_mode: DrawMode, difficulty: Difficulty, winnable: bool) -> Self {
        Self {
            draw_mode,
            difficulty,
            stock: Vec::new(),
            waste: Vec::new(),
            foundations: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            tableau: [
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
            redeals_used: 0,
            max_passes: Difficulty::default_max_passes(),
            game_over: false,
            winnable_deal: winnable,
            win_bonus_awarded: false,
        }
    }

    /// Configure the Hard-mode redeal allowance (default 3). Test-only;
    /// production always uses `Difficulty::default_max_passes`.
    #[cfg(test)]
    pub fn with_max_passes(mut self, n: u32) -> Self {
        self.max_passes = n;
        self
    }

    #[cfg(test)]
    pub fn set_max_passes(&mut self, n: u32) {
        self.max_passes = n;
    }

    /// Deal a fresh random game.
    pub fn reset(&mut self) {
        self.winnable_deal = false;
        let deck = shuffled_deck();
        self.deal_from(deck);
    }

    /// Deal a fresh winnable-slanted game.
    pub fn reset_winnable(&mut self) {
        self.winnable_deal = true;
        let deck = winnable_deck();
        self.deal_from(deck);
    }

    /// Core deal: pop 28 cards into tableau piles (pile i gets i+1 cards,
    /// top face-up), remainder becomes the stock. Deck end = next dealt.
    fn deal_from(&mut self, mut deck: Vec<Card>) {
        debug_assert!(deck.len() == 52, "deal needs a full 52-card deck");
        for (i, pile) in self.tableau.iter_mut().enumerate() {
            pile.clear();
            for _ in 0..=i {
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
        self.waste.clear();
        for f in &mut self.foundations {
            f.clear();
        }
        self.score = 0;
        self.moves = 0;
        self.redeals_used = 0;
        self.game_over = false;
        self.win_bonus_awarded = false;
    }

    // ---- accessors ----

    pub fn score(&self) -> i32 {
        self.score
    }

    pub fn move_count(&self) -> u32 {
        self.moves
    }

    /// Test-only introspection; production reads `redeals_remaining` instead.
    #[cfg(test)]
    pub fn redeals_used(&self) -> u32 {
        self.redeals_used
    }

    #[cfg(test)]
    pub fn max_passes(&self) -> u32 {
        self.max_passes
    }

    /// Test-only introspection; production doesn't currently surface this.
    #[cfg(test)]
    pub fn is_winnable_deal(&self) -> bool {
        self.winnable_deal
    }

    pub fn is_game_over(&self) -> bool {
        self.game_over || self.is_won()
    }

    pub fn draw_count(&self) -> usize {
        match self.draw_mode {
            DrawMode::Draw1 => 1,
            DrawMode::Draw3 => 3,
        }
    }

    /// Effective redeal limit: `None` = unlimited (Easy/Normal).
    pub fn redeal_limit(&self) -> Option<u32> {
        if self.difficulty.unlimited_redeals() {
            None
        } else {
            Some(self.max_passes)
        }
    }

    pub fn redeals_remaining(&self) -> Option<u32> {
        self.redeal_limit()
            .map(|limit| limit.saturating_sub(self.redeals_used))
    }

    pub fn can_redeal(&self) -> bool {
        if self.is_won() || self.game_over {
            return false;
        }
        match self.redeal_limit() {
            None => true,
            Some(limit) => self.redeals_used < limit,
        }
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

    /// Draw up to `draw_count` cards stock -> waste; recycle waste -> stock
    /// when empty. Returns false when the draw was refused (game over on
    /// exhausted Hard-mode passes, already won, or nothing to draw).
    pub fn draw_from_stock(&mut self) -> bool {
        if self.game_over || self.is_won() {
            return false;
        }
        if self.stock.is_empty() {
            if self.waste.is_empty() {
                return false;
            }
            if !self.can_redeal() {
                // Hard mode: passes exhausted -> game over.
                self.game_over = true;
                return false;
            }
            while let Some(mut c) = self.waste.pop() {
                c.face_up = false;
                self.stock.push(c);
            }
            self.redeals_used += 1;
            self.moves += 1;
            self.score = self.score.saturating_sub(SCORE_REDEAL_PENALTY).max(0);
            return true;
        }
        for _ in 0..self.draw_count() {
            if let Some(mut c) = self.stock.pop() {
                c.face_up = true;
                self.waste.push(c);
            } else {
                break;
            }
        }
        self.moves += 1;
        true
    }

    // ---- placement rules ----

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

    /// Tableau rule: empty takes a King; otherwise face-up, alternating
    /// colors, one rank lower.
    pub fn can_place_on_tableau(&self, card: Card, tableau: usize) -> bool {
        let pile = match self.tableau.get(tableau) {
            Some(p) => p,
            None => return false,
        };
        match pile.last() {
            None => card.rank.is_king(),
            Some(top) => {
                top.face_up
                    && top.color() != card.color()
                    && top.rank.value() == card.rank.value() + 1
            }
        }
    }

    /// Check a moving stack is a valid face-up alternating-descending sequence.
    pub fn is_valid_tableau_sequence(cards: &[Card]) -> bool {
        is_valid_descending_alternating_run(cards)
    }

    /// Index of the first face-up card in a tableau pile (start of the
    /// movable fan), if any.
    fn first_face_up(&self, pile: usize) -> Option<usize> {
        self.tableau.get(pile)?.iter().position(|c| c.face_up)
    }

    /// Largest grabbable suffix length of a tableau pile: the biggest valid
    /// face-up alternating-descending fan starting at/after the first
    /// face-up card. Used by the TUI cursor grab.
    pub fn grab_count_tableau(&self, pile: usize) -> usize {
        let p = match self.tableau.get(pile) {
            Some(p) => p,
            None => return 0,
        };
        let first = match self.first_face_up(pile) {
            Some(f) => f,
            None => return 0,
        };
        let len = p.len();
        for start in first..len {
            if Self::is_valid_tableau_sequence(&p[start..]) {
                return len - start;
            }
        }
        0
    }

    /// Flip a newly exposed tableau top face-up (+score). Returns true if flipped.
    fn reveal_tableau_top(&mut self, pile: usize) -> bool {
        if let Some(top) = self.tableau.get_mut(pile).and_then(|p| p.last_mut()) {
            if !top.face_up {
                top.face_up = true;
                self.score += SCORE_FLIP_TABLEAU;
                return true;
            }
        }
        false
    }

    fn on_card_moved_to_foundation(&mut self) {
        self.score += SCORE_TO_FOUNDATION;
        self.moves += 1;
        self.check_win_bonus();
    }

    fn check_win_bonus(&mut self) {
        if self.is_won() && !self.win_bonus_awarded {
            self.win_bonus_awarded = true;
            self.score += SCORE_WIN_BONUS;
        }
    }

    // ---- standard moves ----

    /// Waste top -> any legal foundation.
    pub fn move_waste_to_foundation(&mut self) -> bool {
        let card = match self.waste_top() {
            Some(c) => c,
            None => return false,
        };
        let target = (0..4).find(|&f| self.can_place_on_foundation(card, f));
        match target {
            Some(f) => {
                self.waste.pop();
                self.foundations[f].push(card);
                self.on_card_moved_to_foundation();
                true
            }
            None => false,
        }
    }

    /// Waste top -> tableau pile.
    pub fn move_waste_to_tableau(&mut self, tableau: usize) -> bool {
        let card = match self.waste_top() {
            Some(c) => c,
            None => return false,
        };
        if tableau >= 7 || !self.can_place_on_tableau(card, tableau) {
            return false;
        }
        self.waste.pop();
        self.tableau[tableau].push(card);
        self.score += SCORE_WASTE_TO_TABLEAU;
        self.moves += 1;
        true
    }

    /// Tableau pile top -> any legal foundation.
    pub fn move_tableau_to_foundation(&mut self, tableau: usize) -> bool {
        if tableau >= 7 {
            return false;
        }
        let card = match self.tableau_top(tableau) {
            Some(c) if c.face_up => c,
            _ => return false,
        };
        let target = (0..4).find(|&f| self.can_place_on_foundation(card, f));
        match target {
            Some(f) => {
                self.tableau[tableau].pop();
                self.foundations[f].push(card);
                self.reveal_tableau_top(tableau);
                self.on_card_moved_to_foundation();
                true
            }
            None => false,
        }
    }

    /// Move `count` face-up cards from one tableau pile to another
    /// (alternating-color descending sequences, incl. multi-card moves).
    pub fn move_tableau_to_tableau(&mut self, from: usize, to: usize, count: usize) -> bool {
        if from >= 7 || to >= 7 || from == to || count == 0 {
            return false;
        }
        let len = self.tableau[from].len();
        if count > len {
            return false;
        }
        let start = len - count;
        // The moved fan must start at/after the first face-up card and be valid.
        match self.first_face_up(from) {
            Some(first) if start >= first => {}
            _ => return false,
        }
        if !Self::is_valid_tableau_sequence(&self.tableau[from][start..]) {
            return false;
        }
        let moving_bottom = self.tableau[from][start];
        if !self.can_place_on_tableau(moving_bottom, to) {
            return false;
        }
        let mut moving: Vec<Card> = self.tableau[from].drain(start..).collect();
        self.tableau[to].append(&mut moving);
        self.reveal_tableau_top(from);
        self.score += SCORE_TABLEAU_TO_TABLEAU;
        self.moves += 1;
        true
    }

    /// Foundation top -> tableau pile (costs points).
    pub fn move_foundation_to_tableau(&mut self, foundation: usize, tableau: usize) -> bool {
        if foundation >= 4 || tableau >= 7 {
            return false;
        }
        let card = match self.foundation_top(foundation) {
            Some(c) => c,
            None => return false,
        };
        if !self.can_place_on_tableau(card, tableau) {
            return false;
        }
        self.foundations[foundation].pop();
        self.tableau[tableau].push(card);
        self.score = self
            .score
            .saturating_add(SCORE_FOUNDATION_TO_TABLEAU)
            .max(0);
        self.moves += 1;
        true
    }

    /// Repeatedly move waste/tableau tops to foundations until no progress.
    /// Returns the number of cards moved.
    pub fn auto_finish(&mut self) -> usize {
        let mut moved = 0;
        loop {
            let waste_ready = self
                .waste_top()
                .map(|c| (0..4).any(|f| self.can_place_on_foundation(c, f)))
                .unwrap_or(false);
            if waste_ready {
                self.move_waste_to_foundation();
                moved += 1;
                continue;
            }
            let pile = (0..7).find(|&t| {
                self.tableau_top(t)
                    .map(|c| c.face_up && (0..4).any(|f| self.can_place_on_foundation(c, f)))
                    .unwrap_or(false)
            });
            match pile {
                Some(t) => {
                    self.move_tableau_to_foundation(t);
                    moved += 1;
                }
                None => break,
            }
        }
        moved
    }

    /// Whether every card is on the foundations.
    pub fn is_won(&self) -> bool {
        self.foundations.iter().map(|f| f.len()).sum::<usize>() == 52
    }

    /// True when at least one legal action exists (any move, draw, or
    /// redeal). Test-only; production relies on `hint_text` returning `None`
    /// to signal "no move found" instead.
    #[cfg(test)]
    pub fn has_legal_moves(&self) -> bool {
        if self.is_won() || self.game_over {
            return false;
        }
        if !self.stock.is_empty()
            || (self.stock.is_empty() && !self.waste.is_empty() && self.can_redeal())
        {
            return true;
        }
        if let Some(w) = self.waste_top() {
            if (0..4).any(|f| self.can_place_on_foundation(w, f)) {
                return true;
            }
            if (0..7).any(|t| self.can_place_on_tableau(w, t)) {
                return true;
            }
        }
        for t in 0..7 {
            if let Some(c) = self.tableau_top(t) {
                if c.face_up {
                    if (0..4).any(|f| self.can_place_on_foundation(c, f)) {
                        return true;
                    }
                    // Any stack starting at/after first face-up to another pile.
                    if let Some(first) = self.first_face_up(t) {
                        let len = self.tableau[t].len();
                        for start in first..len {
                            let bottom = self.tableau[t][start];
                            if (0..7).any(|d| {
                                d != t
                                    && Self::is_valid_tableau_sequence(&self.tableau[t][start..])
                                    && self.can_place_on_tableau(bottom, d)
                            }) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        for f in 0..4 {
            if let Some(c) = self.foundation_top(f) {
                if (0..7).any(|t| self.can_place_on_tableau(c, t)) {
                    return true;
                }
            }
        }
        false
    }

    /// Suggest the next move (Easy-mode winnable hint). Priority: foundation
    /// moves, tableau consolidation, waste to tableau, draw/redeal.
    pub fn hint(&self) -> Option<Hint> {
        if self.is_won() || self.game_over {
            return None;
        }
        if let Some(w) = self.waste_top() {
            if (0..4).any(|f| self.can_place_on_foundation(w, f)) {
                return Some(Hint::WasteToFoundation);
            }
        }
        for t in 0..7 {
            if let Some(c) = self.tableau_top(t) {
                if c.face_up && (0..4).any(|f| self.can_place_on_foundation(c, f)) {
                    return Some(Hint::TableauToFoundation(t));
                }
            }
        }
        if self.can_auto_finish_safely() {
            return Some(Hint::AutoFinish);
        }
        // Tableau consolidation that exposes a face-down card.
        for from in 0..7 {
            if let Some(first) = self.first_face_up(from) {
                if first == 0 {
                    continue; // nothing to expose
                }
                let len = self.tableau[from].len();
                for start in first..len {
                    if !Self::is_valid_tableau_sequence(&self.tableau[from][start..]) {
                        continue;
                    }
                    let bottom = self.tableau[from][start];
                    if let Some(to) =
                        (0..7).find(|&d| d != from && self.can_place_on_tableau(bottom, d))
                    {
                        return Some(Hint::TableauToTableau {
                            from,
                            to,
                            count: len - start,
                        });
                    }
                }
            }
        }
        if let Some(w) = self.waste_top() {
            if let Some(t) = (0..7).find(|&t| self.can_place_on_tableau(w, t)) {
                return Some(Hint::WasteToTableau(t));
            }
        }
        if !self.stock.is_empty() {
            return Some(Hint::Draw);
        }
        if !self.waste.is_empty() && self.can_redeal() {
            return Some(Hint::Redeal);
        }
        // Last resort: pull a foundation card back to unlock tableau.
        for f in 0..4 {
            if let Some(c) = self.foundation_top(f) {
                if let Some(t) = (0..7).find(|&t| self.can_place_on_tableau(c, t)) {
                    return Some(Hint::FoundationToTableau {
                        foundation: f,
                        tableau: t,
                    });
                }
            }
        }
        None
    }

    /// Hint text for UI status lines, if any move is suggested.
    pub fn hint_text(&self) -> Option<String> {
        self.hint().map(|h| h.describe())
    }

    /// True when stock + waste are empty and every tableau card is face-up:
    /// auto-finish cannot strand a needed card.
    fn can_auto_finish_safely(&self) -> bool {
        self.stock.is_empty()
            && self.waste.is_empty()
            && self.tableau.iter().flatten().all(|c| c.face_up)
    }
}

// ---- deals ----

/// Winnable-slanted 52-card deck (deck end = first dealt).
///
/// Construction: tableau tops are the lowest cards (4 aces + 3 twos) so they
/// leave for the foundations immediately; face-down fillers are the highest
/// cards (they can wait on tableau until their foundation turn); the stock
/// holds the middle ranks in ascending draw order with each draw-3 triple
/// reversed so the most-needed card always lands on top of the waste.
/// A greedy "foundation move else draw" player always finishes this deal.
fn winnable_deck() -> Vec<Card> {
    // Suit rotation per rank keeps colors mixed.
    fn suits_for_rank(rank: u8) -> [Suit; 4] {
        let base = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];
        let rot = (rank as usize) % 4;
        [
            base[rot],
            base[(rot + 1) % 4],
            base[(rot + 2) % 4],
            base[(rot + 3) % 4],
        ]
    }

    // Tableau tops: 4 aces + 3 twos (leave one two for the stock).
    let mut tops: Vec<Card> = Vec::with_capacity(7);
    for &s in &suits_for_rank(1) {
        tops.push(Card::new(s, Rank(1)));
    }
    let mut stock_two: Option<Card> = None;
    for (i, &s) in suits_for_rank(2).iter().enumerate() {
        let c = Card::new(s, Rank(2));
        if i == 3 {
            stock_two = Some(c);
        } else {
            tops.push(c);
        }
    }

    // Face-down fillers: 21 highest remaining cards (K,Q,J,10,9 x4 + one 8),
    // stored ASCENDING (index 0 = lowest). Each pile drains bottom-to-top in
    // ascending rank order, so a flipped card's same-suit predecessor is never
    // buried beneath a higher unplayable card: predecessors are tableau tops
    // (played first), earlier stock cards (drawn in ascending order), or
    // already-played cards. Hence greedy "foundation move else draw" play
    // always finishes this deal with no tableau consolidation needed.
    let mut high: Vec<Card> = Vec::with_capacity(21);
    // The one 8 not needed elsewhere: pick the first suit of rank 8.
    high.push(Card::new(suits_for_rank(8)[0], Rank(8)));
    for rank in [9u8, 10, 11, 12, 13] {
        for &s in &suits_for_rank(rank) {
            high.push(Card::new(s, Rank(rank)));
        }
    }
    let mut fillers: Vec<Vec<Card>> = vec![Vec::new(); 7];
    for (i, pile) in fillers.iter_mut().enumerate().skip(1) {
        // Bottom fillers: highest remaining (deepest = highest).
        for _ in 0..(i - 1) {
            pile.push(high.pop().expect("filler stock"));
        }
        // Top filler: lowest remaining (flips first, playable soonest).
        let top_filler = high.remove(0);
        pile.push(top_filler);
    }
    debug_assert!(high.is_empty());

    // Stock: every card not on the tableau, ascending, triples reversed.
    let mut used = std::collections::HashSet::new();
    for c in tops.iter().chain(fillers.iter().flatten()) {
        used.insert((c.suit, c.rank));
    }
    // Note: `stock_two` stays out of `used` so it lands in the stock as the
    // first card drawn (all aces are tableau tops, so rank 2 leads).
    let mut rest: Vec<Card> = Vec::with_capacity(24);
    for rank in 1u8..=13 {
        for &s in &suits_for_rank(rank) {
            if !used.contains(&(s, Rank(rank))) {
                rest.push(Card::new(s, Rank(rank)));
            }
        }
    }
    assert_eq!(rest.len(), 24, "winnable stock must hold 24 cards");
    debug_assert!(rest.contains(&stock_two.expect("fourth two")));
    debug_assert!(rest
        .windows(2)
        .all(|w| w[0].rank.value() <= w[1].rank.value()));
    // Draw order = ascending; reverse within each triple of 3 so the lowest
    // card of each batch lands on top of the waste (works for draw-1 too,
    // where later draws cover earlier ones).
    let mut draw_order: Vec<Card> = Vec::with_capacity(24);
    for chunk in rest.chunks(3) {
        for c in chunk.iter().rev() {
            draw_order.push(*c);
        }
    }
    // Deck vec: stock first (bottom), tableau last (top = dealt first).
    // Stock end must be the LAST drawn, i.e. reverse of draw order.
    let mut deck: Vec<Card> = Vec::with_capacity(52);
    for c in draw_order.iter().rev() {
        deck.push(*c);
    }
    // Tableau: pile 0 ends up on top. Push piles in reverse; within a pile
    // push the top first so fillers pop first.
    for i in (0..7).rev() {
        deck.push(tops[i]);
        for c in fillers[i].iter().rev() {
            deck.push(*c);
        }
    }
    assert_eq!(deck.len(), 52);
    deck
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn all_52_unique(game: &KlondikeGame) -> bool {
        let mut seen = HashSet::new();
        let cards = game
            .stock
            .iter()
            .chain(game.waste.iter())
            .chain(game.foundations.iter().flatten())
            .chain(game.tableau.iter().flatten());
        let mut n = 0;
        for c in cards {
            n += 1;
            seen.insert((c.suit, c.rank));
        }
        n == 52 && seen.len() == 52
    }

    #[test]
    fn deal_layout_is_valid() {
        let game = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal);
        for (i, pile) in game.tableau.iter().enumerate() {
            assert_eq!(pile.len(), i + 1, "pile {i} size");
            assert!(pile.last().unwrap().face_up, "pile {i} top face-up");
            assert!(
                pile[..i].iter().all(|c| !c.face_up),
                "pile {i} fillers face-down"
            );
        }
        assert_eq!(game.stock.len(), 24);
        assert!(game.waste.is_empty());
        assert!(game.foundations.iter().all(|f| f.is_empty()));
        assert!(all_52_unique(&game));
        assert_eq!(game.score(), 0);
        assert_eq!(game.move_count(), 0);
    }

    #[test]
    fn draw_modes_and_recycle() {
        let mut g1 = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        assert!(g1.draw_from_stock());
        assert_eq!(g1.waste.len(), 1);
        assert_eq!(g1.stock.len(), 23);

        let mut g3 = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal);
        assert!(g3.draw_from_stock());
        assert_eq!(g3.waste.len(), 3);
        assert_eq!(g3.stock.len(), 21);

        // Drain stock then recycle.
        while !g3.stock.is_empty() {
            assert!(g3.draw_from_stock());
        }
        let waste_len = g3.waste.len();
        assert_eq!(waste_len, 24);
        assert!(g3.draw_from_stock()); // recycle
        assert_eq!(g3.stock.len(), 24);
        assert!(g3.waste.is_empty());
        assert_eq!(g3.redeals_used(), 1);
    }

    #[test]
    fn hard_mode_limits_passes_then_game_over() {
        let mut game = KlondikeGame::new(DrawMode::Draw3, Difficulty::Hard);
        assert_eq!(game.redeal_limit(), Some(3));
        assert_eq!(game.redeals_remaining(), Some(3));
        // Exhaust the stock.
        while !game.stock.is_empty() {
            assert!(game.draw_from_stock());
        }
        for expected in [2, 1, 0] {
            assert!(game.draw_from_stock(), "redeal should succeed");
            assert_eq!(game.redeals_remaining(), Some(expected));
            while !game.stock.is_empty() {
                assert!(game.draw_from_stock());
            }
        }
        assert!(!game.can_redeal());
        assert!(!game.draw_from_stock(), "4th redeal refused");
        assert!(game.is_game_over());
    }

    #[test]
    fn easy_and_normal_have_unlimited_redeals() {
        let easy = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        let normal = KlondikeGame::new(DrawMode::Draw3, Difficulty::Normal);
        assert_eq!(easy.redeal_limit(), None);
        assert_eq!(normal.redeal_limit(), None);
        assert!(easy.can_redeal() && normal.can_redeal());
    }

    #[test]
    fn foundation_rules() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        let ace_spades = Card::new_face_up(Suit::Spades, Rank::ACE);
        assert!(game.can_place_on_foundation(ace_spades, 0));
        let two_spades = Card::new_face_up(Suit::Spades, Rank(2));
        assert!(!game.can_place_on_foundation(two_spades, 0)); // empty needs ace
        game.foundations[0].push(ace_spades);
        assert!(game.can_place_on_foundation(two_spades, 0));
        let two_hearts = Card::new_face_up(Suit::Hearts, Rank(2));
        assert!(!game.can_place_on_foundation(two_hearts, 0)); // wrong suit
        assert!(!game.can_place_on_foundation(two_spades, 9)); // bad index
    }

    #[test]
    fn tableau_rules_alternating_descending() {
        let game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        let king = Card::new_face_up(Suit::Spades, Rank::KING);
        let queen_hearts = Card::new_face_up(Suit::Hearts, Rank::QUEEN);
        let queen_spades = Card::new_face_up(Suit::Spades, Rank::QUEEN);
        // Empty pile takes a king (pile index checked against real piles, so
        // test via a cleared local: use can_place on pile after clearing).
        let mut g = game;
        g.tableau[0].clear();
        assert!(g.can_place_on_tableau(king, 0));
        assert!(!g.can_place_on_tableau(queen_hearts, 0)); // only kings on empty
        g.tableau[0].push(king);
        assert!(g.can_place_on_tableau(queen_hearts, 0)); // red on black
        assert!(!g.can_place_on_tableau(queen_spades, 0)); // same color rejected
        assert!(!g.can_place_on_tableau(Card::new_face_up(Suit::Hearts, Rank::JACK), 0)); // rank gap
        assert!(!g.can_place_on_tableau(king, 9)); // bad index
    }

    #[test]
    fn waste_to_foundation_and_tableau() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.stock.clear();
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.waste.clear();
        game.waste.push(Card::new_face_up(Suit::Clubs, Rank::ACE));
        let before_moves = game.move_count();
        assert!(game.move_waste_to_foundation());
        assert_eq!(game.waste.len(), 0);
        assert_eq!(game.score(), SCORE_TO_FOUNDATION);
        assert_eq!(game.move_count(), before_moves + 1);

        // Waste 5♦ onto tableau 6♣.
        game.waste.push(Card::new_face_up(Suit::Diamonds, Rank(5)));
        game.tableau[0].push(Card::new_face_up(Suit::Clubs, Rank(6)));
        assert!(game.move_waste_to_tableau(0));
        assert_eq!(game.tableau[0].len(), 2);
    }

    #[test]
    fn tableau_to_foundation_flips_and_scores() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.tableau[0].push(Card::new(Suit::Hearts, Rank(5))); // face-down
        game.tableau[0].push(Card::new_face_up(Suit::Diamonds, Rank::ACE));
        let score0 = game.score();
        assert!(game.move_tableau_to_foundation(0));
        assert_eq!(game.tableau[0].len(), 1);
        assert!(game.tableau[0][0].face_up, "exposed card flips");
        assert_eq!(
            game.score(),
            score0 + SCORE_TO_FOUNDATION + SCORE_FLIP_TABLEAU
        );
    }

    #[test]
    fn multi_card_tableau_move() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.tableau.iter_mut().for_each(|p| p.clear());
        // Source pile: 9♣ 8♥ 7♣ (valid sequence) over a face-down card.
        game.tableau[0].push(Card::new(Suit::Spades, Rank::ACE));
        game.tableau[0].push(Card::new_face_up(Suit::Clubs, Rank(9)));
        game.tableau[0].push(Card::new_face_up(Suit::Hearts, Rank(8)));
        game.tableau[0].push(Card::new_face_up(Suit::Clubs, Rank(7)));
        // Dest pile top: 10♥.
        game.tableau[1].push(Card::new_face_up(Suit::Hearts, Rank(10)));
        assert!(game.move_tableau_to_tableau(0, 1, 3));
        assert_eq!(game.tableau[1].len(), 4);
        assert_eq!(game.tableau[0].len(), 1);
        assert!(
            game.tableau[0][0].face_up,
            "source flips after full fan moves"
        );

        // Same-color sequence rejected.
        let mut bad = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        bad.tableau.iter_mut().for_each(|p| p.clear());
        bad.tableau[0].push(Card::new_face_up(Suit::Clubs, Rank(9)));
        bad.tableau[0].push(Card::new_face_up(Suit::Spades, Rank(8))); // black on black
        bad.tableau[1].push(Card::new_face_up(Suit::Hearts, Rank(10)));
        assert!(!bad.move_tableau_to_tableau(0, 1, 2));
    }

    #[test]
    fn foundation_to_tableau_costs_points() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.foundations[0].push(Card::new_face_up(Suit::Spades, Rank::ACE));
        game.foundations[0].push(Card::new_face_up(Suit::Spades, Rank(2)));
        game.tableau[0].push(Card::new_face_up(Suit::Hearts, Rank(3)));
        // Need score first so the -15 is observable (floor is 0).
        game.score = 50;
        assert!(game.move_foundation_to_tableau(0, 0));
        assert_eq!(game.score(), 50 + SCORE_FOUNDATION_TO_TABLEAU);
        assert_eq!(game.tableau[0].len(), 2);
    }

    #[test]
    fn win_detection_and_bonus() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        // Stack 51 cards on foundations (indexed by suit discriminant
        // order S,H,D,C = 0..3), hold back K♣ in waste.
        let rest = vec![Card::new_face_up(Suit::Clubs, Rank::KING)];
        let mut per_suit: [Vec<Card>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for &suit in &Suit::all() {
            for v in 1..=13 {
                if suit == Suit::Clubs && v == 13 {
                    continue;
                }
                per_suit[suit as usize].push(Card::new_face_up(suit, Rank(v)));
            }
        }
        game.foundations = per_suit;
        game.stock.clear();
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.waste = rest;
        assert!(!game.is_won());
        assert!(game.move_waste_to_foundation());
        assert!(game.is_won());
        assert!(game.is_game_over());
        assert!(game.score() >= SCORE_TO_FOUNDATION + SCORE_WIN_BONUS);
    }

    #[test]
    fn auto_finish_moves_available_cards() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.stock.clear();
        game.waste.clear();
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.tableau[0].push(Card::new_face_up(Suit::Spades, Rank::ACE));
        game.tableau[1].push(Card::new_face_up(Suit::Hearts, Rank::ACE));
        let moved = game.auto_finish();
        assert_eq!(moved, 2);
        assert_eq!(game.foundations.iter().map(|f| f.len()).sum::<usize>(), 2);
    }

    #[test]
    fn winnable_deal_is_valid_and_solvable() {
        for mode in [DrawMode::Draw1, DrawMode::Draw3] {
            let mut game = KlondikeGame::new_winnable(mode, Difficulty::Easy);
            assert!(game.is_winnable_deal());
            assert!(all_52_unique(&game));
            assert_eq!(game.tableau[6].len(), 7);
            assert_eq!(game.stock.len(), 24);
            // Greedy: foundation move else draw, until won.
            let mut steps = 0;
            while !game.is_won() && steps < 600 {
                steps += 1;
                if game.move_waste_to_foundation() {
                    continue;
                }
                let t = (0..7).find(|&t| {
                    game.tableau_top(t)
                        .map(|c| c.face_up && (0..4).any(|f| game.can_place_on_foundation(c, f)))
                        .unwrap_or(false)
                });
                if let Some(t) = t {
                    assert!(game.move_tableau_to_foundation(t));
                    continue;
                }
                assert!(
                    game.draw_from_stock(),
                    "solver stalled in {mode:?} (won={})",
                    game.is_won()
                );
            }
            assert!(game.is_won(), "winnable deal should solve in {mode:?}");
            assert!(game.score() > 0);
            assert!(game.move_count() > 0);
        }
    }

    #[test]
    fn hint_suggests_foundation_move_on_winnable_deal() {
        let game = KlondikeGame::new_winnable(DrawMode::Draw1, Difficulty::Easy);
        match game.hint() {
            Some(Hint::TableauToFoundation(_)) | Some(Hint::WasteToFoundation) => {}
            other => panic!("expected foundation hint, got {other:?}"),
        }
    }

    #[test]
    fn difficulty_helpers_legal_moves_and_descriptions() {
        assert_eq!(Difficulty::Easy.recommended_draw_mode(), DrawMode::Draw1);
        assert_eq!(Difficulty::Normal.recommended_draw_mode(), DrawMode::Draw3);
        assert_eq!(Difficulty::Hard.recommended_draw_mode(), DrawMode::Draw3);

        let game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        assert!(game.has_legal_moves());
        assert!(!game.is_game_over());

        // Configurable Hard pass limit.
        let mut hard = KlondikeGame::new(DrawMode::Draw3, Difficulty::Hard).with_max_passes(1);
        assert_eq!(hard.max_passes(), 1);
        hard.set_max_passes(2);
        assert_eq!(hard.max_passes(), 2);
        assert_eq!(hard.redeals_remaining(), Some(2));

        // Hint descriptions render.
        let hinted = KlondikeGame::new_winnable(DrawMode::Draw1, Difficulty::Easy);
        let hint = hinted.hint().expect("winnable deal has a hint");
        assert!(!hint.describe().is_empty());
        assert!(!hinted.hint_text().map(|s| s.is_empty()).unwrap_or(true));
    }

    #[test]
    fn black_four_onto_red_five_is_legal() {
        // Reported case: dropping 4♣ onto 5♦ is a legal tableau move
        // (alternating colors, one rank down), from a tableau pile and
        // from the waste.
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.stock.clear();
        game.waste.clear();
        game.tableau[0].push(Card::new_face_up(Suit::Diamonds, Rank(5)));
        game.tableau[1].push(Card::new_face_up(Suit::Clubs, Rank(4)));
        assert!(game.can_place_on_tableau(Card::new_face_up(Suit::Clubs, Rank(4)), 0));
        assert!(game.move_tableau_to_tableau(1, 0, 1));
        assert_eq!(game.tableau[0].len(), 2);

        game.tableau[2].push(Card::new_face_up(Suit::Diamonds, Rank(5)));
        game.waste.push(Card::new_face_up(Suit::Clubs, Rank(4)));
        assert!(game.move_waste_to_tableau(2));
        assert_eq!(game.tableau[2].len(), 2);
    }

    #[test]
    fn tableau_rejects_same_color_gap_and_ascending() {
        // Look-alikes of 4♣/5♦ that must stay illegal: same color, rank
        // gap, ascending order, non-king on empty, face-down destination.
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        game.tableau.iter_mut().for_each(|p| p.clear());
        game.tableau[0].push(Card::new_face_up(Suit::Clubs, Rank(5)));
        assert!(!game.can_place_on_tableau(Card::new_face_up(Suit::Spades, Rank(4)), 0));
        game.tableau[1].push(Card::new_face_up(Suit::Diamonds, Rank(5)));
        assert!(!game.can_place_on_tableau(Card::new_face_up(Suit::Clubs, Rank(3)), 1));
        game.tableau[2].push(Card::new_face_up(Suit::Clubs, Rank(4)));
        assert!(!game.can_place_on_tableau(Card::new_face_up(Suit::Diamonds, Rank(5)), 2));
        assert!(!game.can_place_on_tableau(Card::new_face_up(Suit::Clubs, Rank(4)), 3));
        assert!(game.can_place_on_tableau(Card::new_face_up(Suit::Spades, Rank::KING), 3));
        game.tableau[4].push(Card::new(Suit::Diamonds, Rank(5)));
        assert!(!game.can_place_on_tableau(Card::new_face_up(Suit::Clubs, Rank(4)), 4));
    }

    #[test]
    fn score_and_move_count_track_actions() {
        let mut game = KlondikeGame::new(DrawMode::Draw1, Difficulty::Easy);
        let m0 = game.move_count();
        assert!(game.draw_from_stock());
        assert_eq!(game.move_count(), m0 + 1);
        assert_eq!(game.score(), 0); // draws are free
    }
}
