//! Desktop-style rendering: title bar, toolbar, felt table, card widgets,
//! modal dialogs. Rendering also records clickable regions for the mouse.

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Action, App, CursorArea, GameMode, Hit, HitRegion, Screen};
use crate::cards::Card;
use crate::rules::{Difficulty, cards};

// ---- palette ----

/// Felt table background.
const FELT: Color = Color::Rgb(14, 80, 44);
/// Near-black ink for text on light surfaces.
const INK: Color = Color::Rgb(15, 23, 42);
/// Secondary ink on light surfaces.
const MUTED: Color = Color::Rgb(71, 85, 105);
/// Window title / status bar background.
const TITLE_BG: Color = Color::Rgb(17, 24, 39);
/// Toolbar background (light) and foreground.
const BAR_BG: Color = Color::Rgb(226, 232, 240);
const BAR_FG: Color = Color::Rgb(15, 23, 42);
const BAR_SEP: Color = Color::Rgb(148, 163, 184);
/// Modal dialog background.
const DIALOG_BG: Color = Color::Rgb(248, 250, 252);
/// Selected-row highlight inside dialogs.
const SELECT_BG: Color = Color::Rgb(191, 219, 254);
/// Card face ink.
const CARD_RED: Color = Color::Rgb(185, 28, 28);
const CARD_BLACK: Color = Color::Rgb(17, 24, 39);
/// Card back: solid blue with a lighter pattern.
const BACK_BG: Color = Color::Rgb(30, 58, 138);
const BACK_PATTERN: Color = Color::Rgb(147, 197, 253);
/// Empty-slot outline and ghost art on the felt.
const SLOT_BORDER: Color = Color::Rgb(110, 168, 138);
const GHOST: Color = Color::Rgb(151, 196, 172);
/// Focus rings.
const NORMAL_RING: Color = Color::Rgb(100, 116, 139);
const CURSOR_RING: Color = Color::White;
const GRABBED_RING: Color = Color::Yellow;
const TARGET_RING: Color = Color::Rgb(74, 222, 128);
/// Status/hint message ink on felt.
const MSG_FG: Color = Color::Rgb(254, 240, 138);
const DIM_ON_FELT: Color = Color::Rgb(203, 213, 225);
/// "No useful moves" banner.
const ALERT_BG: Color = Color::Rgb(153, 27, 27);

/// Card widget height in rows. Width varies per game (7 or 9).
const CARD_H: u16 = 5;
/// Rows used by the title bar, toolbar and info strip.
const CHROME_TOP: u16 = 3;
/// Smallest table height: chrome, top row, a column header plus one card,
/// message strip and status bar.
const MIN_HEIGHT: u16 = CHROME_TOP + CARD_H + 1 + 1 + CARD_H + 1 + 2;
/// Menu dialog size.
const MENU_W: u16 = 64;
const MENU_H: u16 = 19;

// ---- render entry ----

/// Render the whole app and return the clickable regions of this frame.
pub fn render(frame: &mut Frame, app: &App) -> Vec<HitRegion> {
    let area = frame.area();
    let mut canvas = Canvas {
        buf: frame.buffer_mut(),
        hits: Vec::new(),
    };
    canvas.fill(area, Style::default().bg(FELT));
    match app.screen {
        Screen::Menu => render_menu(&mut canvas, area, app),
        Screen::Game => render_game(&mut canvas, area, app),
    }
    if app.show_help {
        render_help_overlay(&mut canvas, area);
    } else if app.screen == Screen::Game && app.is_won() && fits_game(area, app) {
        render_win_overlay(&mut canvas, area, app);
    }
    if !app.truecolor {
        downsample_colors(canvas.buf);
    }
    canvas.hits
}

fn fmt_time(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Minimum terminal width for a game: its widest row of cards.
fn min_width(app: &App) -> u16 {
    let row = |n: u16, w: u16| n * w + (n - 1);
    match app.mode {
        GameMode::Klondike => row(7, 9),
        GameMode::FreeCell => {
            let (n, w) = freecell_top_geometry(app);
            row(8, 9).max(row(n, w))
        }
        GameMode::Spider => row(10, 7),
    }
}

fn fits_game(area: Rect, app: &App) -> bool {
    area.width >= min_width(app) && area.height >= MIN_HEIGHT
}

/// Number of top-row slots and their width in FreeCell (narrower cards
/// when 5 free cells crowd the row).
fn freecell_top_geometry(app: &App) -> (u16, u16) {
    let n = u16::try_from(app.freecell.game.num_cells + 4).unwrap_or(u16::MAX);
    (n, if n > 8 { 7 } else { 9 })
}

// ---- canvas: clipped painting + hit regions ----

struct Canvas<'a> {
    buf: &'a mut Buffer,
    hits: Vec<HitRegion>,
}

impl Canvas<'_> {
    fn fill(&mut self, rect: Rect, style: Style) {
        let rect = rect.intersection(self.buf.area);
        for pos in rect.positions() {
            self.buf[pos].set_symbol(" ").set_style(style);
        }
    }

    /// Write text clipped to the buffer.
    fn put(&mut self, x: u16, y: u16, text: &str, style: Style) {
        let area = self.buf.area;
        if y < area.top() || y >= area.bottom() || x < area.left() || x >= area.right() {
            return;
        }
        let clipped = truncate_cells(text, usize::from(area.right() - x));
        self.buf.set_string(x, y, clipped, style);
    }

    /// Center text in `rect` at row `y`, truncating to the rect width.
    fn put_centered(&mut self, rect: Rect, y: u16, text: &str, style: Style) {
        let clipped = truncate_cells(text, usize::from(rect.width));
        let pad = rect.width.saturating_sub(cell_width(clipped));
        self.put(rect.x + pad / 2, y, clipped, style);
    }

    fn hit(&mut self, rect: Rect, hit: Hit) {
        let rect = rect.intersection(self.buf.area);
        if !rect.is_empty() {
            self.hits.push(HitRegion { rect, hit });
        }
    }

    /// A clickable label: paints `text` at (x, y) and records its region.
    fn button(&mut self, x: u16, y: u16, text: &str, style: Style, hit: Hit) -> u16 {
        let w = cell_width(text);
        self.put(x, y, text, style);
        self.hit(Rect::new(x, y, w, 1), hit);
        w
    }
}

/// Longest prefix of `s` that fits in `max` terminal cells.
fn truncate_cells(s: &str, max: usize) -> &str {
    let mut width = 0;
    for (i, ch) in s.char_indices() {
        width += ch.width().unwrap_or(0);
        if width > max {
            return &s[..i];
        }
    }
    s
}

fn cell_width(s: &str) -> u16 {
    u16::try_from(s.width()).unwrap_or(u16::MAX)
}

// ---- focus state ----

#[derive(Clone, Copy, PartialEq)]
enum PileState {
    Normal,
    Cursor,
    Grabbed,
    Target,
}

/// Border/label state for a pile: grabbed source first, then legal target,
/// then cursor, otherwise plain. `targets` is computed once per frame.
fn pile_state(
    app: &App,
    targets: &[(CursorArea, usize)],
    area: CursorArea,
    index: usize,
) -> PileState {
    if app
        .selected
        .is_some_and(|sel| sel.area == area && sel.index == index)
    {
        PileState::Grabbed
    } else if targets.contains(&(area, index)) {
        PileState::Target
    } else if app.cursor.area == area && app.cursor.index == index {
        PileState::Cursor
    } else {
        PileState::Normal
    }
}

fn ring_color(state: PileState) -> Color {
    match state {
        PileState::Normal => NORMAL_RING,
        PileState::Cursor => CURSOR_RING,
        PileState::Grabbed => GRABBED_RING,
        PileState::Target => TARGET_RING,
    }
}

fn card_ink(card: Card) -> Color {
    if card.is_red() { CARD_RED } else { CARD_BLACK }
}

fn back_pattern(inner: usize) -> String {
    (0..inner)
        .map(|i| if i % 2 == 0 { '▓' } else { ' ' })
        .collect()
}

// ---- card widgets ----

impl Canvas<'_> {
    /// Rounded box outline rows (top and bottom) for a widget of width `w`.
    fn box_edges(&mut self, x: u16, y: u16, w: u16, style: Style) {
        let inner = "─".repeat(usize::from(w - 2));
        self.put(x, y, &format!("╭{inner}╮"), style);
        self.put(x, y + CARD_H - 1, &format!("╰{inner}╯"), style);
    }

    /// One interior row: side borders plus `body`.
    fn box_row(&mut self, x: u16, y: u16, w: u16, body: &str, edge: Style, fill: Style) {
        self.put(x, y, "│", edge);
        self.put(x + 1, y, body, fill);
        self.put(x + w - 1, y, "│", edge);
    }

    /// Face-up card as a 5-row widget with rounded corners.
    fn card_face(&mut self, x: u16, y: u16, w: u16, card: Card, ring: Color) {
        let inner = usize::from(w.saturating_sub(2));
        if inner < 3 {
            return;
        }
        let ink = card_ink(card);
        let border = Style::default().fg(ring).bg(Color::White);
        let face = Style::default()
            .fg(ink)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD);
        let center = Style::default().fg(ink).bg(Color::White);
        let tag = card.to_string();
        let pad = (inner - 1) / 2;
        let middle = format!(
            "{}{}{}",
            " ".repeat(pad),
            card.suit.symbol(),
            " ".repeat(inner - pad - 1)
        );
        self.box_edges(x, y, w, border);
        self.box_row(x, y + 1, w, &format!("{tag:<inner$}"), border, face);
        self.box_row(x, y + 2, w, &middle, border, center);
        self.box_row(x, y + 3, w, &format!("{tag:>inner$}"), border, face);
    }

    /// Face-down card: solid back with a lighter crosshatch.
    fn card_back(&mut self, x: u16, y: u16, w: u16, ring: Color) {
        let inner = usize::from(w.saturating_sub(2));
        if inner < 3 {
            return;
        }
        let border = Style::default().fg(ring).bg(BACK_BG);
        let pattern = Style::default().fg(BACK_PATTERN).bg(BACK_BG);
        let row = back_pattern(inner);
        self.box_edges(x, y, w, border);
        for dy in 1..CARD_H - 1 {
            self.box_row(x, y + dy, w, &row, border, pattern);
        }
    }

    /// Compressed 1-row strip for a buried card in a tall column, so every
    /// buried rank stays visible.
    fn card_strip(&mut self, x: u16, y: u16, w: u16, card: Card, ring: Color) {
        let inner = usize::from(w.saturating_sub(2));
        if inner < 3 {
            return;
        }
        if card.face_up {
            let edge = Style::default().fg(ring).bg(Color::White);
            let text = Style::default()
                .fg(card_ink(card))
                .bg(Color::White)
                .add_modifier(Modifier::BOLD);
            let tag = card.to_string();
            self.box_row(x, y, w, &format!("{tag:<inner$}"), edge, text);
        } else {
            let edge = Style::default().fg(ring).bg(BACK_BG);
            let pattern = Style::default().fg(BACK_PATTERN).bg(BACK_BG);
            self.box_row(x, y, w, &back_pattern(inner), edge, pattern);
        }
    }

    /// Empty slot: thin outline plus a centered ghost glyph.
    fn slot(&mut self, x: u16, y: u16, w: u16, ghost: &str, ring: Color) {
        let inner = usize::from(w.saturating_sub(2));
        if inner < 2 {
            return;
        }
        let border = Style::default().fg(ring).bg(FELT);
        let art = Style::default().fg(GHOST).bg(FELT);
        let ghost = truncate_cells(ghost, inner);
        let gw = usize::from(cell_width(ghost));
        let pad = (inner - gw) / 2;
        let middle = format!("{}{ghost}{}", " ".repeat(pad), " ".repeat(inner - pad - gw));
        let blank = " ".repeat(inner);
        self.box_edges(x, y, w, border);
        self.box_row(x, y + 1, w, &blank, border, art);
        self.box_row(x, y + 2, w, &middle, border, art);
        self.box_row(x, y + 3, w, &blank, border, art);
    }

    fn pile_card(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        card: Option<Card>,
        ghost: &str,
        state: PileState,
    ) {
        let ring = ring_color(state);
        match card {
            Some(c) if c.face_up => self.card_face(x, y, w, c, ring),
            Some(_) => self.card_back(x, y, w, ring),
            None => self.slot(x, y, w, ghost, ring),
        }
    }

    /// Pile caption under (or above) a card widget. Focus states get a chip.
    fn caption(&mut self, x: u16, y: u16, w: u16, text: &str, state: PileState) {
        let style = match state {
            PileState::Normal => Style::default().fg(DIM_ON_FELT).bg(FELT),
            PileState::Cursor => Style::default()
                .fg(INK)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
            PileState::Grabbed => Style::default()
                .fg(Color::Black)
                .bg(GRABBED_RING)
                .add_modifier(Modifier::BOLD),
            PileState::Target => Style::default()
                .fg(Color::Black)
                .bg(TARGET_RING)
                .add_modifier(Modifier::BOLD),
        };
        let row = Rect::new(x, y, w, 1);
        self.fill(row, style);
        self.put_centered(row, y, text, style);
    }

    /// A top-row pile: card widget, caption and click region.
    #[allow(clippy::too_many_arguments)]
    fn top_pile(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        card: Option<Card>,
        ghost: &str,
        caption: &str,
        state: PileState,
        area: CursorArea,
        index: usize,
    ) {
        self.pile_card(x, y, w, card, ghost, state);
        self.caption(x, y + CARD_H, w, caption, state);
        self.hit(
            Rect::new(x, y, w, CARD_H + 1),
            Hit::Pile {
                area,
                index,
                card: None,
            },
        );
    }
}

// ---- window chrome: title bar, toolbar, info, message, status ----

fn render_title_bar(c: &mut Canvas, area: Rect, app: &App) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    c.fill(bar, Style::default().bg(TITLE_BG));
    // macOS-style traffic lights: the strongest desktop cue in one row.
    for (dx, color) in [(1, Color::Red), (3, Color::Yellow), (5, Color::Green)] {
        c.put(
            area.x + dx,
            area.y,
            "●",
            Style::default().fg(color).bg(TITLE_BG),
        );
    }
    let title_style = Style::default()
        .fg(Color::White)
        .bg(TITLE_BG)
        .add_modifier(Modifier::BOLD);
    c.put(area.x + 8, area.y, "Solitaire", title_style);
    if app.screen == Screen::Menu {
        c.put_centered(
            bar,
            area.y,
            "— New game —",
            Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
        );
        return;
    }
    let doc = format!("— {} · {} —", app.mode, app.difficulty());
    c.put_centered(
        bar,
        area.y,
        &doc,
        Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
    );
    let game = app.game();
    let stats = format!(
        "Score {}   Moves {}   {}",
        game.score(),
        game.move_count(),
        fmt_time(app.elapsed().as_secs())
    );
    let w = cell_width(&stats);
    c.put(
        bar.right().saturating_sub(w + 1),
        area.y,
        &stats,
        Style::default().fg(Color::White).bg(TITLE_BG),
    );
}

fn toolbar_actions(app: &App) -> &'static [(&'static str, &'static str, Action)] {
    match app.mode {
        GameMode::Klondike => &[
            ("New", "n", Action::New),
            ("Restart", "r", Action::Restart),
            ("Undo", "u", Action::Undo),
            ("Draw", "d", Action::Draw),
            ("Auto", "a", Action::Auto),
            ("Hint", "H", Action::Hint),
        ],
        GameMode::FreeCell => &[
            ("New", "n", Action::New),
            ("Restart", "r", Action::Restart),
            ("Undo", "u", Action::Undo),
            ("Auto", "a", Action::Auto),
            ("Hint", "H", Action::Hint),
        ],
        GameMode::Spider => &[
            ("New", "n", Action::New),
            ("Restart", "r", Action::Restart),
            ("Undo", "u", Action::Undo),
            ("Deal", "d", Action::Draw),
            ("Hint", "H", Action::Hint),
        ],
    }
}

fn render_toolbar(c: &mut Canvas, area: Rect, app: &App) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    c.fill(bar, Style::default().bg(BAR_BG));
    let plain = Style::default().fg(BAR_FG).bg(BAR_BG);
    let key_style = plain.add_modifier(Modifier::BOLD);
    let right: [(&str, Action); 3] = [
        ("Menu m", Action::Menu),
        ("Help ?", Action::Help),
        ("Quit q", Action::Quit),
    ];
    let right_w: u16 = right.iter().map(|(t, _)| cell_width(t) + 3).sum();
    let left_end = bar.right().saturating_sub(right_w + 1);

    let mut x = area.x + 1;
    for &(label, key, action) in toolbar_actions(app) {
        let w = cell_width(label) + 1 + cell_width(key);
        if x + w + 3 > left_end {
            break;
        }
        c.put(x, area.y, label, plain);
        c.put(x + cell_width(label) + 1, area.y, key, key_style);
        c.hit(Rect::new(x, area.y, w, 1), Hit::Action(action));
        x += w + 1;
        c.put(x, area.y, "│", Style::default().fg(BAR_SEP).bg(BAR_BG));
        x += 2;
    }
    let mut x = left_end;
    for (text, action) in right {
        x += 2 + c.button(x, area.y, text, plain, Hit::Action(action)) + 1;
    }
}

fn mode_extra_text(app: &App) -> String {
    match app.mode {
        GameMode::Klondike => {
            let g = &app.klondike.game;
            let redeals = g
                .redeals_remaining()
                .map(|n| format!(" · Redeals left {n}"))
                .unwrap_or_default();
            format!(
                "Stock {} · Waste {} · {}{redeals}",
                g.stock.len(),
                g.waste.len(),
                g.draw_mode
            )
        }
        GameMode::FreeCell => format!(
            "Free cells {}/{}",
            app.freecell.game.free_count(),
            app.freecell.game.num_cells
        ),
        GameMode::Spider => format!(
            "{} · Completed {}/8 · {}",
            deals_left(app.spider.game.stock_deals_remaining()),
            app.spider.game.completed_sequences,
            app.spider.game.suits
        ),
    }
}

fn render_info_strip(c: &mut Canvas, area: Rect, app: &App) {
    let style = Style::default().fg(DIM_ON_FELT).bg(FELT);
    let text = format!("Deal #{} · {}", app.game().deal(), mode_extra_text(app));
    c.put(area.x + 2, area.y, &text, style);
    let undo = format!("Undo {}", app.undo_depth());
    c.put(
        area.right().saturating_sub(cell_width(&undo) + 2),
        area.y,
        &undo,
        style,
    );
}

/// Status message row. While a confirmation is pending it becomes a prompt;
/// when no useful move remains it becomes a clickable alert banner.
fn render_message_strip(c: &mut Canvas, area: Rect, app: &App) {
    let row = Rect::new(area.x, area.y, area.width, 1);
    let max = usize::from(area.width.saturating_sub(4));
    if app.confirm.is_some() {
        let style = Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        c.fill(row, style);
        c.put(
            area.x + 1,
            area.y,
            truncate_cells(&app.status_message, max),
            style,
        );
        return;
    }
    if app.is_stuck() {
        let style = Style::default().fg(Color::White).bg(ALERT_BG);
        let bold = style.add_modifier(Modifier::BOLD);
        c.fill(row, style);
        let mut x = area.x + 1;
        x += c.button(
            x,
            area.y,
            "No useful moves left.",
            bold,
            Hit::Action(Action::Hint),
        ) + 2;
        for (text, action) in [
            ("[u undo]", Action::Undo),
            ("[r restart]", Action::Restart),
            ("[n new deal]", Action::New),
        ] {
            x += c.button(x, area.y, text, style, Hit::Action(action)) + 1;
        }
        return;
    }
    c.put(
        area.x + 1,
        area.y,
        "●",
        Style::default()
            .fg(TARGET_RING)
            .bg(FELT)
            .add_modifier(Modifier::BOLD),
    );
    c.put(
        area.x + 3,
        area.y,
        truncate_cells(&app.status_message, max),
        Style::default().fg(MSG_FG).bg(FELT),
    );
}

/// Bottom status bar: `left` context, key hints on the right.
fn render_status_bar(c: &mut Canvas, area: Rect, left: &str, right: &str) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    c.fill(bar, Style::default().bg(TITLE_BG));
    let right_w = cell_width(right);
    let room = if bar.width > right_w + 4 {
        c.put(
            bar.right().saturating_sub(right_w + 1),
            area.y,
            right,
            Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
        );
        bar.width - right_w - 3
    } else {
        bar.width.saturating_sub(2)
    };
    c.put(
        area.x + 1,
        area.y,
        truncate_cells(left, usize::from(room)),
        Style::default().fg(Color::White).bg(TITLE_BG),
    );
}

fn cursor_context(app: &App) -> String {
    let show = |c: Option<Card>| c.map_or_else(|| "empty".to_string(), |c| c.to_string());
    let index = app.cursor.index;
    match app.cursor.area {
        CursorArea::Stock => match app.mode {
            GameMode::Klondike => format!("Stock · {} left", cards(app.klondike.game.stock.len())),
            GameMode::Spider => format!(
                "Stock · {}",
                deals_left(app.spider.game.stock_deals_remaining())
            ),
            GameMode::FreeCell => "Stock".to_string(),
        },
        CursorArea::Waste => format!("Waste · {}", show(app.klondike.game.waste_top())),
        CursorArea::Foundation => {
            let top = match app.mode {
                GameMode::Klondike => app.klondike.game.foundation_top(index),
                GameMode::FreeCell => app.freecell.game.foundation_top(index),
                GameMode::Spider => None,
            };
            format!("Foundation {} · {}", index + 1, show(top))
        }
        CursorArea::FreeCell => {
            format!(
                "Cell {} · {}",
                index + 1,
                show(app.freecell.game.cell_card(index))
            )
        }
        CursorArea::Tableau => {
            let pile = match app.mode {
                GameMode::Klondike => &app.klondike.game.tableau[index],
                GameMode::FreeCell => &app.freecell.game.tableau[index],
                GameMode::Spider => &app.spider.game.tableau[index],
            };
            format!(
                "Tableau {} · {} · {}",
                index + 1,
                cards(pile.len()),
                show(pile.last().copied())
            )
        }
    }
}

// ---- game screen ----

fn deals_left(n: usize) -> String {
    if n == 1 {
        "1 deal left".to_string()
    } else {
        format!("{n} deals left")
    }
}

fn render_game(c: &mut Canvas, area: Rect, app: &App) {
    if !fits_game(area, app) {
        render_too_small(c, area, min_width(app), MIN_HEIGHT, &app.mode.to_string());
        return;
    }
    render_title_bar(c, area, app);
    render_toolbar(c, Rect::new(area.x, area.y + 1, area.width, 1), app);
    render_info_strip(c, Rect::new(area.x, area.y + 2, area.width, 1), app);

    // Computed once per frame rather than once per pile widget.
    let targets = app.legal_targets();
    let top = Rect::new(area.x, area.y + CHROME_TOP, area.width, CARD_H + 1);
    match app.mode {
        GameMode::Klondike => render_klondike_top(c, app, &targets, top),
        GameMode::FreeCell => render_freecell_top(c, app, &targets, top),
        GameMode::Spider => render_spider_top(c, app, &targets, top),
    }

    let tab_y = top.bottom();
    let msg_y = area.bottom() - 2;
    let fans = Rect::new(area.x, tab_y, area.width, msg_y - tab_y - 1);
    match app.mode {
        GameMode::Klondike => render_fans(c, app, &targets, fans, &app.klondike.game.tableau, 9),
        GameMode::FreeCell => render_fans(c, app, &targets, fans, &app.freecell.game.tableau, 9),
        GameMode::Spider => render_fans(c, app, &targets, fans, &app.spider.game.tableau, 7),
    }

    render_message_strip(c, Rect::new(area.x, msg_y, area.width, 1), app);
    render_status_bar(
        c,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        &cursor_context(app),
        "Space select · ? help",
    );
}

fn render_too_small(c: &mut Canvas, area: Rect, w: u16, h: u16, what: &str) {
    c.fill(area, Style::default().bg(TITLE_BG));
    let style = Style::default().fg(Color::White).bg(TITLE_BG);
    let mid = area.y + area.height / 2;
    c.put_centered(
        area,
        mid,
        &format!("Enlarge the window to play {what}"),
        style,
    );
    c.put_centered(
        area,
        mid + 1,
        &format!(
            "(needs {w}×{h}, have {}×{}) · q quits",
            area.width, area.height
        ),
        style,
    );
}

/// Center `n` card widgets of width `w` in `area`, returning their x offsets.
fn center_slots(area: Rect, n: u16, w: u16) -> Vec<u16> {
    if n == 0 {
        return Vec::new();
    }
    let gap = if n > 1 {
        (area.width.saturating_sub(n * w) / (n - 1)).clamp(1, 4)
    } else {
        0
    };
    let total = n * w + gap * (n - 1);
    let x0 = area.x + area.width.saturating_sub(total) / 2;
    (0..n).map(|i| x0 + i * (w + gap)).collect()
}

// ---- top rows ----

fn render_klondike_top(c: &mut Canvas, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let g = &app.klondike.game;
    let w = 9;
    let xs = center_slots(area, 6, w);
    let state = |a, i| pile_state(app, targets, a, i);

    let stock_state = state(CursorArea::Stock, 0);
    let stock_card = g.stock.last().copied();
    let ghost = if g.waste.is_empty() || !g.can_redeal() {
        "·"
    } else {
        "↻"
    };
    c.top_pile(
        xs[0],
        area.y,
        w,
        stock_card,
        ghost,
        &format!("Stock {}", g.stock.len()),
        stock_state,
        CursorArea::Stock,
        0,
    );
    c.top_pile(
        xs[1],
        area.y,
        w,
        g.waste_top(),
        "–",
        "Waste",
        state(CursorArea::Waste, 0),
        CursorArea::Waste,
        0,
    );
    for f in 0..4 {
        c.top_pile(
            xs[2 + f],
            area.y,
            w,
            g.foundation_top(f),
            "A",
            &format!("F{}", f + 1),
            state(CursorArea::Foundation, f),
            CursorArea::Foundation,
            f,
        );
    }
}

fn render_freecell_top(c: &mut Canvas, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let g = &app.freecell.game;
    let (n, w) = freecell_top_geometry(app);
    let xs = center_slots(area, n, w);
    for (i, &x) in xs.iter().enumerate().take(g.num_cells) {
        c.top_pile(
            x,
            area.y,
            w,
            g.cell_card(i),
            "·",
            &format!("C{}", i + 1),
            pile_state(app, targets, CursorArea::FreeCell, i),
            CursorArea::FreeCell,
            i,
        );
    }
    for f in 0..4 {
        c.top_pile(
            xs[g.num_cells + f],
            area.y,
            w,
            g.foundation_top(f),
            "A",
            &format!("F{}", f + 1),
            pile_state(app, targets, CursorArea::Foundation, f),
            CursorArea::Foundation,
            f,
        );
    }
}

fn render_spider_top(c: &mut Canvas, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let g = &app.spider.game;
    let w = 9;
    let xs = center_slots(area, 3, w);
    let deals = g.stock_deals_remaining();
    c.top_pile(
        xs[0],
        area.y,
        w,
        g.stock.last().copied(),
        "·",
        &format!("Stock ×{deals}"),
        pile_state(app, targets, CursorArea::Stock, 0),
        CursorArea::Stock,
        0,
    );
    c.slot(
        xs[1],
        area.y,
        w,
        &format!("{}/8", g.completed_sequences),
        SLOT_BORDER,
    );
    c.caption(xs[1], area.y + CARD_H, w, "Completed", PileState::Normal);
    c.slot(xs[2], area.y, w, &g.suits.to_string(), SLOT_BORDER);
    c.caption(xs[2], area.y + CARD_H, w, "Suits", PileState::Normal);
}

// ---- tableau fans ----

/// Row offsets for a pile's cards: roomy overlapping widgets when the
/// column fits (face-up cards at stride 2 so rank strips stay visible,
/// face-downs collapsed to a 1-row edge), else 1-row strips for every card.
/// Returns the offsets and whether the compact layout was used.
fn fan_offsets(pile: &[Card], room: u16) -> (Vec<u16>, bool) {
    let roomy: Vec<u16> = pile
        .iter()
        .scan(0u16, |y, card| {
            let at = *y;
            *y += if card.face_up { 2 } else { 1 };
            Some(at)
        })
        .collect();
    if roomy.last().is_none_or(|&last| last + CARD_H <= room) {
        (roomy, false)
    } else {
        ((0..).take(pile.len()).collect(), true)
    }
}

/// Render tableau columns as overlapping desktop-style card fans.
fn render_fans(
    c: &mut Canvas,
    app: &App,
    targets: &[(CursorArea, usize)],
    area: Rect,
    piles: &[Vec<Card>],
    card_w: u16,
) {
    let n = u16::try_from(piles.len()).expect("pile count fits u16");
    let xs = center_slots(area, n, card_w);
    let fan_y = area.y + 1;
    let room = area.bottom() - fan_y;

    for (i, (pile, &x)) in piles.iter().zip(&xs).enumerate() {
        let col_state = pile_state(app, targets, CursorArea::Tableau, i);
        let column = Rect::new(x, area.y, card_w, area.height);
        // The whole column is a drop target; cards are hit regions on top.
        c.hit(
            column,
            Hit::Pile {
                area: CursorArea::Tableau,
                index: i,
                card: None,
            },
        );
        // Header: the column's keyboard shortcut (1–9, then 0).
        let key = ((i + 1) % 10).to_string();
        c.caption(x, area.y, card_w, &key, col_state);

        if pile.is_empty() {
            let ghost = if app.mode == GameMode::Klondike {
                "K"
            } else {
                ""
            };
            c.slot(x, fan_y, card_w, ghost, ring_color(col_state));
            continue;
        }
        let (offsets, compact) = fan_offsets(pile, room);
        let last = pile.len() - 1;
        // Overflowing columns anchor to the pile top (the playable end).
        let shift = (offsets[last] + CARD_H).saturating_sub(room);
        let grabbed = match app.selected {
            Some(sel) if sel.area == CursorArea::Tableau && sel.index == i => sel.count,
            _ => 0,
        };
        let mut hidden = 0;
        for (j, (&card, &off)) in pile.iter().zip(&offsets).enumerate() {
            if off < shift {
                hidden += 1;
                continue;
            }
            let y = fan_y + off - shift;
            let ring = if j + grabbed > last {
                GRABBED_RING
            } else {
                ring_color(col_state)
            };
            if compact && j < last {
                c.card_strip(x, y, card_w, card, ring);
            } else if card.face_up {
                c.card_face(x, y, card_w, card, ring);
            } else {
                c.card_back(x, y, card_w, ring);
            }
            let next = offsets.get(j + 1).map_or(off + CARD_H, |&o| o.max(shift));
            c.hit(
                Rect::new(x, y, card_w, next - off),
                Hit::Pile {
                    area: CursorArea::Tableau,
                    index: i,
                    card: Some(j),
                },
            );
        }
        if hidden > 0 {
            let style = Style::default().fg(Color::Black).bg(Color::Yellow);
            let badge = Rect::new(x, fan_y, card_w, 1);
            c.fill(badge, style);
            c.put_centered(badge, fan_y, &format!("▲{hidden}"), style);
        }
    }
}

// ---- menu (desktop new-game window) ----

fn render_menu(c: &mut Canvas, area: Rect, app: &App) {
    if area.width < MENU_W + 2 || area.height < MENU_H + 3 {
        render_too_small(c, area, MENU_W + 2, MENU_H + 3, "Solitaire");
        return;
    }
    render_title_bar(c, area, app);
    render_status_bar(
        c,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        &app.status_message,
        "",
    );
    let body = dialog_frame(
        c,
        area,
        (MENU_W, MENU_H),
        "Solitaire — New game",
        "Esc close",
    );

    let blurbs = [
        "7 piles · stock + waste · 4 foundations",
        "8 piles · free cells · every card visible",
        "10 piles · deal rows · clear K→A runs",
    ];
    let mut y = body.y;
    for (mode, blurb) in GameMode::ALL.into_iter().zip(blurbs) {
        let active = mode == app.menu.game;
        let bg = if active { SELECT_BG } else { DIALOG_BG };
        let row = Rect::new(body.x, y, body.width, 2);
        c.fill(row, Style::default().bg(bg));
        let bold = Style::default().fg(INK).bg(bg).add_modifier(Modifier::BOLD);
        c.put(body.x + 2, y, if active { "▸" } else { " " }, bold);
        c.put(
            body.x + 4,
            y,
            &format!("{}  {mode}", mode.index() + 1),
            bold,
        );
        c.put(body.x + 4, y + 1, blurb, Style::default().fg(MUTED).bg(bg));
        c.hit(row, Hit::MenuGame(mode));
        y += 2;
    }

    y += 1;
    let heading = Style::default()
        .fg(INK)
        .bg(DIALOG_BG)
        .add_modifier(Modifier::BOLD);
    c.put(body.x + 2, y, "Difficulty", heading);
    y += 1;
    let mut x = body.x + 4;
    for d in Difficulty::ALL {
        let active = d == app.menu.difficulty;
        let style = if active {
            heading
        } else {
            Style::default().fg(MUTED).bg(DIALOG_BG)
        };
        let label = format!("{} {d}", if active { "(●)" } else { "(○)" });
        x += c.button(x, y, &label, style, Hit::MenuDifficulty(d)) + 4;
    }
    y += 2;
    let note = Style::default().fg(MUTED).bg(DIALOG_BG);
    let draw_line = format!(
        "Klondike: {} (d toggles) · Easy deals are solvable",
        app.menu_draw_mode()
    );
    c.button(body.x + 2, y, &draw_line, note, Hit::MenuDraw);
    c.put(
        body.x + 2,
        y + 1,
        "FreeCell cells: Easy 5 · Normal 4 · Hard 3",
        note,
    );
    c.put(
        body.x + 2,
        y + 2,
        "Spider suits: Easy 1 · Normal 2 · Hard 4",
        note,
    );

    let btn = if app.menu_resumes() {
        "[  Resume · Enter  ]"
    } else {
        "[  Deal · Enter  ]"
    };
    let btn_x = body.x + body.width.saturating_sub(cell_width(btn)) / 2;
    let btn_style = Style::default()
        .fg(Color::White)
        .bg(TITLE_BG)
        .add_modifier(Modifier::BOLD);
    c.button(btn_x, body.bottom() - 2, btn, btn_style, Hit::MenuStart);
    c.put_centered(
        Rect::new(body.x, body.bottom() - 1, body.width, 1),
        body.bottom() - 1,
        "↑↓ game · ←→ difficulty · d draw · Enter · Esc back",
        note,
    );
}

// ---- overlays ----

/// Centered modal: drop shadow, light body, dark title strip. Returns the
/// interior body rect. Callers check the screen is large enough.
fn dialog_frame(c: &mut Canvas, area: Rect, size: (u16, u16), title: &str, extra: &str) -> Rect {
    let (width, height) = size;
    let outer = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let shadow = Rect {
        x: outer.x + 1,
        y: outer.y + 1,
        ..outer
    };
    c.fill(shadow, Style::default().bg(Color::Black));
    c.fill(outer, Style::default().bg(DIALOG_BG));
    c.fill(Rect { height: 1, ..outer }, Style::default().bg(TITLE_BG));
    c.put(
        outer.x + 2,
        outer.y,
        title,
        Style::default()
            .fg(Color::White)
            .bg(TITLE_BG)
            .add_modifier(Modifier::BOLD),
    );
    if !extra.is_empty() {
        c.put(
            outer.right().saturating_sub(cell_width(extra) + 2),
            outer.y,
            extra,
            Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
        );
    }
    Rect::new(
        outer.x + 2,
        outer.y + 2,
        width - 4,
        height.saturating_sub(4),
    )
}

const HELP_ROWS: [(&str, &str); 16] = [
    ("← → ↑ ↓ / hjkl", "move along a row · jump between rows"),
    ("Tab / Shift-Tab", "cycle pile areas"),
    ("Space / Enter", "grab · place · draw on the stock"),
    ("+ / -", "hold more / fewer cards of a run"),
    ("F", "send the cursor card to a foundation"),
    ("d", "draw (Klondike) · deal a row (Spider)"),
    ("a", "auto-move safe cards · finish"),
    ("u", "undo"),
    ("H", "hint"),
    ("1–9, 0", "jump to tableau column"),
    ("s w f c t", "stock · waste · foundation · cells · tableau"),
    ("n / r", "new deal · restart this deal"),
    ("m", "menu · change game + difficulty"),
    ("q / Esc", "cancel grab · quit"),
    ("Mouse", "click to grab/place · click a card to grab"),
    ("", "from it · right-click sends a card up"),
];

fn render_help_overlay(c: &mut Canvas, area: Rect) {
    let (w, h) = (64, 21);
    if area.width < w + 2 || area.height < h + 1 {
        c.fill(
            Rect::new(area.x, area.y, area.width, 1),
            Style::default().bg(TITLE_BG),
        );
        c.put(
            area.x + 1,
            area.y,
            "Help: enlarge the window · any key closes",
            Style::default().fg(Color::White).bg(TITLE_BG),
        );
        return;
    }
    let body = dialog_frame(c, area, (w, h), "Help — Keyboard & mouse", "any key closes");
    let bold = Style::default()
        .fg(INK)
        .bg(DIALOG_BG)
        .add_modifier(Modifier::BOLD);
    for (y, (key, desc)) in (body.y..body.bottom()).zip(HELP_ROWS) {
        c.put(body.x, y, key, bold);
        c.put(body.x + 18, y, desc, Style::default().fg(INK).bg(DIALOG_BG));
    }
}

fn render_win_overlay(c: &mut Canvas, area: Rect, app: &App) {
    let body = dialog_frame(c, area, (54, 9), "You win!", "");
    let game = app.game();
    c.put_centered(
        body,
        body.y,
        &format!(
            "{} · {} · Deal #{}",
            app.mode,
            app.difficulty(),
            game.deal()
        ),
        Style::default()
            .fg(INK)
            .bg(DIALOG_BG)
            .add_modifier(Modifier::BOLD),
    );
    c.put_centered(
        body,
        body.y + 1,
        &format!(
            "Score {} in {} moves ({})",
            game.score(),
            game.move_count(),
            fmt_time(app.elapsed().as_secs())
        ),
        Style::default().fg(MUTED).bg(DIALOG_BG),
    );
    let style = Style::default()
        .fg(Color::White)
        .bg(TITLE_BG)
        .add_modifier(Modifier::BOLD);
    let buttons = [
        ("[ n new deal ]", Action::New),
        ("[ m menu ]", Action::Menu),
        ("[ q quit ]", Action::Quit),
    ];
    let total: u16 = buttons.iter().map(|(t, _)| cell_width(t) + 3).sum::<u16>() - 3;
    let mut x = body.x + body.width.saturating_sub(total) / 2;
    for (text, action) in buttons {
        x += c.button(x, body.y + 3, text, style, Hit::Action(action)) + 3;
    }
}

// ---- color depth ----

/// Replace 24-bit colors with the nearest xterm-256 entries, for terminals
/// that don't advertise truecolor support.
fn downsample_colors(buf: &mut Buffer) {
    for cell in &mut buf.content {
        cell.fg = to_ansi256(cell.fg);
        cell.bg = to_ansi256(cell.bg);
    }
}

fn to_ansi256(color: Color) -> Color {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let nearest = |v: u8| {
        (0u8..6)
            .min_by_key(|&i| LEVELS[usize::from(i)].abs_diff(v))
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = [
        LEVELS[usize::from(ri)],
        LEVELS[usize::from(gi)],
        LEVELS[usize::from(bi)],
    ];
    let avg = u8::try_from((u16::from(r) + u16::from(g) + u16::from(b)) / 3).unwrap_or(255);
    let gray_step = (avg.saturating_sub(8) / 10).min(23);
    let gray = 8 + 10 * gray_step;
    let dist = |c: [u8; 3]| {
        [r, g, b]
            .iter()
            .zip(c)
            .map(|(&a, b)| u32::from(a.abs_diff(b)).pow(2))
            .sum::<u32>()
    };
    if dist([gray; 3]) < dist(cube) {
        Color::Indexed(232 + gray_step)
    } else {
        Color::Indexed(16 + 36 * ri + 6 * gi + bi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Cursor;
    use ratatui::{Terminal, backend::TestBackend};

    fn table() -> App {
        let mut app = App::new(Difficulty::Normal, None, Some(1));
        app.screen = Screen::Game;
        app
    }

    fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let mut hits = Vec::new();
        terminal
            .draw(|f| hits = render(f, app))
            .expect("render succeeds");
        app.hits = hits;
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn fresh_app_boots_into_start_menu() {
        let mut app = App::new(Difficulty::Normal, None, Some(1));
        assert_eq!(app.screen, Screen::Menu);
        let body = rows(&draw(&mut app, 80, 24)).join("\n");
        assert!(body.contains("New game"), "start menu:\n{body}");
        assert!(body.contains("Difficulty"), "difficulty picker:\n{body}");
        assert!(body.contains("Resume"), "unplayed deal resumes:\n{body}");
    }

    #[test]
    fn game_chrome_has_title_toolbar_and_status_bars() {
        let mut app = table();
        let rows = rows(&draw(&mut app, 80, 24));
        assert!(rows[0].contains("Solitaire") && rows[0].contains("Klondike"));
        assert!(
            rows[1].contains("Undo") && rows[1].contains("Quit"),
            "{}",
            rows[1]
        );
        assert!(rows[2].contains("Deal #1"), "{}", rows[2]);
        assert!(rows[23].contains("Space select"), "{}", rows[23]);
        assert!(rows.join("\n").contains("╭"), "card widgets");
    }

    #[test]
    fn felt_background_covers_the_table() {
        let mut app = table();
        assert_eq!(draw(&mut app, 80, 24)[(0, 5)].bg, FELT);
    }

    #[test]
    fn all_modes_render_at_80x24() {
        for mode in GameMode::ALL {
            let mut app = table();
            app.mode = mode;
            app.ensure_cursor_valid();
            let rows = rows(&draw(&mut app, 80, 24));
            assert!(rows[0].contains(&mode.to_string()), "{mode}: {}", rows[0]);
            assert!(!rows.join("").contains("Enlarge"), "{mode} fits 80x24");
        }
    }

    #[test]
    fn too_small_windows_say_so() {
        let mut app = table();
        assert!(rows(&draw(&mut app, 60, 24)).join("\n").contains("Enlarge"));
        app.mode = GameMode::Spider;
        assert!(
            rows(&draw(&mut app, 78, 24))
                .join("\n")
                .contains("needs 79×18")
        );
        app.screen = Screen::Menu;
        assert!(rows(&draw(&mut app, 40, 12)).join("\n").contains("Enlarge"));
        app.show_help = true;
        draw(&mut app, 30, 10); // no panic
    }

    #[test]
    fn freecell_strip_counts_free_cells() {
        let mut app = table();
        app.mode = GameMode::FreeCell;
        app.freecell.game.freecells[0] = app.freecell.game.tableau[0].pop();
        let rows = rows(&draw(&mut app, 80, 24));
        assert!(rows[2].contains("Free cells 3/4"), "{}", rows[2]);
    }

    #[test]
    fn hit_regions_cover_piles_cards_and_toolbar() {
        let mut app = table();
        draw(&mut app, 80, 24);
        let has = |hit: Hit| app.hits.iter().any(|r| r.hit == hit);
        assert!(has(Hit::Action(Action::Undo)));
        assert!(has(Hit::Pile {
            area: CursorArea::Stock,
            index: 0,
            card: None
        }));
        assert!(has(Hit::Pile {
            area: CursorArea::Tableau,
            index: 6,
            card: Some(6)
        }));
        // Clicking the drawn top card of pile 1 grabs it.
        let top = app
            .hits
            .iter()
            .rev()
            .find(|r| {
                r.hit
                    == Hit::Pile {
                        area: CursorArea::Tableau,
                        index: 0,
                        card: Some(0),
                    }
            })
            .unwrap()
            .rect;
        app.click(top.x + 1, top.y + 1, false);
        assert_eq!(
            app.cursor,
            Cursor {
                area: CursorArea::Tableau,
                index: 0
            }
        );
        assert!(app.selected.is_some());
    }

    #[test]
    fn stuck_banner_replaces_the_message() {
        let mut app = table();
        let g = &mut app.klondike.game;
        g.tableau.iter_mut().for_each(Vec::clear);
        g.stock.clear();
        g.waste.clear();
        g.tableau[0].push(Card::up(crate::cards::Suit::Spades, 9));
        g.tableau[1].push(Card::new(
            crate::cards::Suit::Hearts,
            crate::cards::Rank::KING,
        ));
        g.tableau[1].push(Card::up(crate::cards::Suit::Spades, 8));
        assert!(app.is_stuck());
        let rows = rows(&draw(&mut app, 80, 24));
        assert!(rows[22].contains("No useful moves left"), "{}", rows[22]);
    }

    #[test]
    fn downsampling_maps_rgb_to_indexed() {
        assert_eq!(to_ansi256(Color::Rgb(0, 0, 0)), Color::Indexed(16));
        assert_eq!(to_ansi256(Color::Rgb(255, 255, 255)), Color::Indexed(231));
        assert_eq!(to_ansi256(Color::White), Color::White);
        let mut app = table();
        app.truecolor = false;
        let buffer = draw(&mut app, 80, 24);
        assert!(
            buffer
                .content
                .iter()
                .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..)))
        );
    }

    #[test]
    fn truncation_respects_wide_characters() {
        assert_eq!(truncate_cells("abc", 2), "ab");
        assert_eq!(truncate_cells("日本語", 3), "日");
        assert_eq!(truncate_cells("A♠", 5), "A♠");
    }
}
