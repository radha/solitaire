//! Desktop-style rendering: title bar, toolbar, felt table, card widgets,
//! modal dialogs. No boxed-pile TUI chrome.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Clear,
    Frame,
};

use crate::app::{difficulty_label, App, CursorArea, GameMode, Screen};
use crate::cards::Card;

// ---- palette ----

/// Felt table background.
const FELT: Color = Color::Rgb(14, 80, 44);
/// Near-black ink for text on light surfaces.
const INK: Color = Color::Rgb(15, 23, 42);
/// Window title / status bar background.
const TITLE_BG: Color = Color::Rgb(17, 24, 39);
/// Toolbar background (light) and foreground.
const BAR_BG: Color = Color::Rgb(226, 232, 240);
const BAR_FG: Color = Color::Rgb(15, 23, 42);
/// Modal dialog background.
const DIALOG_BG: Color = Color::Rgb(248, 250, 252);
/// Selected-row highlight inside dialogs.
const SELECT_BG: Color = Color::Rgb(191, 219, 254);
/// Card face ink.
const CARD_RED: Color = Color::Rgb(185, 28, 28);
const CARD_BLACK: Color = Color::Rgb(17, 24, 39);
/// Card back: solid felt-blue with a lighter pattern.
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

/// Card widget height in rows. Width varies per game (7 or 9).
const CARD_H: u16 = 5;

// ---- render entry ----

/// Render the whole app.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    // Paint the felt first so every row has the table background.
    fill(frame.buffer_mut(), area, Style::default().bg(FELT));
    match app.screen {
        Screen::Menu => render_menu(frame, app),
        Screen::Game => render_game(frame, app),
    }
    if app.show_help {
        render_help_overlay(frame);
    } else if app.screen == Screen::Game && app.is_won() {
        render_win_overlay(frame, app);
    }
}

fn fmt_time(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

// ---- low-level paint helpers ----

fn fill(buf: &mut Buffer, rect: Rect, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let blank = " ".repeat(rect.width as usize);
    for y in rect.top()..rect.bottom() {
        buf.set_string(rect.x, y, &blank, style);
    }
}

/// Write text clipped to the buffer. Out-of-bounds rows are skipped.
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    let area = buf.area;
    if y < area.top() || y >= area.bottom() || x >= area.right() {
        return;
    }
    // Clip to the remaining width so wide dialogs never wrap.
    let max = (area.right() - x) as usize;
    let clipped = truncate_cells(text, max);
    buf.set_string(x, y, &clipped, style);
}

/// Center text in a rect at row `y`, truncating to the rect width.
fn put_centered(buf: &mut Buffer, rect: Rect, y: u16, text: &str, style: Style) {
    let clipped = truncate_cells(text, rect.width as usize);
    let pad = rect.width.saturating_sub(cell_width(&clipped));
    let x = rect.x + pad / 2;
    put(buf, x, y, &clipped, style);
}

/// Truncate a string to at most `max` terminal cells (ASCII fast path).
fn truncate_cells(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut width = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let w = unicode_width(ch);
        if width + w > max {
            break;
        }
        width += w;
        out.push(ch);
    }
    out
}

fn cell_width(s: &str) -> u16 {
    s.chars().map(unicode_width).sum::<usize>() as u16
}

fn unicode_width(ch: char) -> usize {
    if ch < '\u{1100}' {
        1
    } else {
        // CJK wide ranges; box drawing and card suits stay narrow.
        match ch {
            '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}' => 2,
            _ => 1,
        }
    }
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
/// then cursor, otherwise plain. `targets` is `app.legal_targets()`, computed
/// once per frame by the caller rather than once per pile.
fn pile_state(
    app: &App,
    targets: &[(CursorArea, usize)],
    area: CursorArea,
    index: usize,
) -> PileState {
    if let Some(sel) = app.selected {
        if sel.area == area && sel.index == index {
            return PileState::Grabbed;
        }
    }
    if targets.iter().any(|&(a, i)| a == area && i == index) {
        return PileState::Target;
    }
    if app.cursor.area == area && app.cursor.index == index {
        return PileState::Cursor;
    }
    PileState::Normal
}

fn ring_color(state: PileState) -> Color {
    match state {
        PileState::Normal => NORMAL_RING,
        PileState::Cursor => CURSOR_RING,
        PileState::Grabbed => GRABBED_RING,
        PileState::Target => TARGET_RING,
    }
}

fn card_ink(card: &Card) -> Color {
    if card.is_red() {
        CARD_RED
    } else {
        CARD_BLACK
    }
}

// ---- card widgets ----

/// Draw a face-up card as a 5-row widget with rounded corners.
fn paint_card_face(buf: &mut Buffer, x: u16, y: u16, w: u16, card: &Card, ring: Color) {
    let inner = w.saturating_sub(2) as usize;
    if inner < 3 {
        return;
    }
    let ink = card_ink(card);
    let border = Style::default().fg(ring).bg(Color::White);
    let face = Style::default()
        .fg(ink)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let center_style = Style::default().fg(ink).bg(Color::White);
    let tag = format!("{}{}", card.rank.label(), card.suit.symbol());
    let top_line = format!("{tag:<inner$}");
    let bottom_line = format!("{tag:>inner$}");
    let suit = card.suit.symbol().to_string();
    let pad = (inner.saturating_sub(1)) / 2;
    let middle_line = format!("{}{}{}", " ".repeat(pad), suit, " ".repeat(inner - pad - 1));

    let top = format!("╭{}╮", "─".repeat(inner));
    let bottom = format!("╰{}╯", "─".repeat(inner));
    put(buf, x, y, &top, border);
    put(buf, x, y + 1, "│", border);
    put(buf, x + 1, y + 1, &top_line, face);
    put(buf, x + w - 1, y + 1, "│", border);
    put(buf, x, y + 2, "│", border);
    put(buf, x + 1, y + 2, &middle_line, center_style);
    put(buf, x + w - 1, y + 2, "│", border);
    put(buf, x, y + 3, "│", border);
    put(buf, x + 1, y + 3, &bottom_line, face);
    put(buf, x + w - 1, y + 3, "│", border);
    put(buf, x, y + 4, &bottom, border);
}

/// Draw a face-down card: solid back with a lighter crosshatch.
fn paint_card_back(buf: &mut Buffer, x: u16, y: u16, w: u16, ring: Color) {
    let inner = w.saturating_sub(2) as usize;
    if inner < 3 {
        return;
    }
    let border = Style::default().fg(ring).bg(BACK_BG);
    let pattern = Style::default().fg(BACK_PATTERN).bg(BACK_BG);
    let top = format!("╭{}╮", "─".repeat(inner));
    let bottom = format!("╰{}╯", "─".repeat(inner));
    let row: String = (0..inner)
        .map(|i| if i % 2 == 0 { '▓' } else { ' ' })
        .collect();
    put(buf, x, y, &top, border);
    for dy in 1..=3 {
        put(buf, x, y + dy, "│", border);
        put(buf, x + 1, y + dy, &row, pattern);
        put(buf, x + w - 1, y + dy, "│", border);
    }
    put(buf, x, y + 4, &bottom, border);
}

/// Draw a compressed 1-row fan strip: rank tile for face-up cards,
/// crosshatch for face-down cards. Used when a column is too tall for full
/// overlapping widgets, so every buried rank stays visible.
fn paint_strip(buf: &mut Buffer, x: u16, y: u16, w: u16, card: &Card, ring: Color) {
    let inner = w.saturating_sub(2) as usize;
    if inner < 3 {
        return;
    }
    if card.face_up {
        let edge = Style::default().fg(ring).bg(Color::White);
        let tag = format!("{}{}", card.rank.label(), card.suit.symbol());
        let line = format!("{tag:<inner$}");
        let text = Style::default()
            .fg(card_ink(card))
            .bg(Color::White)
            .add_modifier(Modifier::BOLD);
        put(buf, x, y, "│", edge);
        put(buf, x + 1, y, &line, text);
        put(buf, x + w - 1, y, "│", edge);
    } else {
        let edge = Style::default().fg(ring).bg(BACK_BG);
        let pattern = Style::default().fg(BACK_PATTERN).bg(BACK_BG);
        let row: String = (0..inner)
            .map(|i| if i % 2 == 0 { '▓' } else { ' ' })
            .collect();
        put(buf, x, y, "│", edge);
        put(buf, x + 1, y, &row, pattern);
        put(buf, x + w - 1, y, "│", edge);
    }
}

/// Draw an empty slot: thin outline plus an optional ghost glyph.
fn paint_slot(buf: &mut Buffer, x: u16, y: u16, w: u16, ghost: &str, ring: Color) {
    let inner = w.saturating_sub(2) as usize;
    if inner < 2 {
        return;
    }
    let border = Style::default().fg(ring).bg(FELT);
    let art = Style::default().fg(GHOST).bg(FELT);
    let top = format!("╭{}╮", "─".repeat(inner));
    let bottom = format!("╰{}╯", "─".repeat(inner));
    let pad = inner.saturating_sub(cell_width(ghost) as usize) / 2;
    let middle = format!(
        "{}{}{}",
        " ".repeat(pad),
        ghost,
        " ".repeat(inner - pad - cell_width(ghost) as usize)
    );
    let blank = " ".repeat(inner);
    put(buf, x, y, &top, border);
    put(buf, x, y + 1, "│", border);
    put(buf, x + 1, y + 1, &blank, art);
    put(buf, x + w - 1, y + 1, "│", border);
    put(buf, x, y + 2, "│", border);
    put(buf, x + 1, y + 2, &middle, art);
    put(buf, x + w - 1, y + 2, "│", border);
    put(buf, x, y + 3, "│", border);
    put(buf, x + 1, y + 3, &blank, art);
    put(buf, x + w - 1, y + 3, "│", border);
    put(buf, x, y + 4, &bottom, border);
}

fn paint_pile_card(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    card: Option<&Card>,
    ghost: &str,
    state: PileState,
) {
    let ring = ring_color(state);
    match card {
        Some(c) if c.face_up => paint_card_face(buf, x, y, w, c, ring),
        Some(_) => paint_card_back(buf, x, y, w, ring),
        None => paint_slot(buf, x, y, w, ghost, ring),
    }
}

/// Pile caption under (or above) a card widget. Focus states get a chip.
fn paint_caption(buf: &mut Buffer, x: u16, y: u16, w: u16, text: &str, state: PileState) {
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
    let clipped = truncate_cells(text, w as usize);
    let pad = w.saturating_sub(cell_width(&clipped));
    let left = x + pad / 2;
    // Clear the caption cell run first so chips look solid.
    put(buf, x, y, &" ".repeat(w as usize), style);
    put(buf, left, y, &clipped, style);
}

// ---- window chrome: title bar, toolbar, info, message, status ----

fn render_title_bar(buf: &mut Buffer, area: Rect, app: &App) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    fill(buf, bar, Style::default().bg(TITLE_BG));
    // macOS-style traffic lights: the strongest desktop cue in one row.
    put(
        buf,
        area.x + 1,
        area.y,
        "●",
        Style::default().fg(Color::Red).bg(TITLE_BG),
    );
    put(
        buf,
        area.x + 3,
        area.y,
        "●",
        Style::default().fg(Color::Yellow).bg(TITLE_BG),
    );
    put(
        buf,
        area.x + 5,
        area.y,
        "●",
        Style::default().fg(Color::Green).bg(TITLE_BG),
    );
    let title_style = Style::default()
        .fg(Color::White)
        .bg(TITLE_BG)
        .add_modifier(Modifier::BOLD);
    put(buf, area.x + 8, area.y, "Solitaire", title_style);
    let doc = match app.screen {
        Screen::Menu => "New game".to_string(),
        Screen::Game => format!(
            "{} · {}",
            app.mode.title(),
            difficulty_label(app.difficulty)
        ),
    };
    let stats = match app.screen {
        Screen::Menu => String::new(),
        Screen::Game => format!(
            "Score {}   Moves {}   {}",
            app.score(),
            app.moves(),
            fmt_time(app.elapsed_secs())
        ),
    };
    let center = format!("— {doc} —");
    put_centered(
        buf,
        bar,
        area.y,
        &center,
        Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
    );
    if !stats.is_empty() {
        let style = Style::default().fg(Color::White).bg(TITLE_BG);
        let w = cell_width(&stats);
        put(
            buf,
            bar.right().saturating_sub(w + 1),
            area.y,
            &stats,
            style,
        );
    }
}

fn toolbar_actions(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.mode {
        GameMode::Klondike => vec![
            ("New", "n"),
            ("Undo", "u"),
            ("Draw", "d"),
            ("Auto", "a"),
            ("Hint", "H"),
        ],
        GameMode::FreeCell => vec![("New", "n"), ("Undo", "u"), ("Auto", "a"), ("Hint", "H")],
        GameMode::SpiderMini => vec![
            ("New", "n"),
            ("Undo", "u"),
            ("Deal", "d"),
            ("Auto", "a"),
            ("Hint", "H"),
        ],
    }
}

fn render_toolbar(buf: &mut Buffer, area: Rect, app: &App) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    fill(buf, bar, Style::default().bg(BAR_BG));
    let plain = Style::default().fg(BAR_FG).bg(BAR_BG);
    let key_style = Style::default()
        .fg(BAR_FG)
        .bg(BAR_BG)
        .add_modifier(Modifier::BOLD);
    let mut x = area.x + 1;
    for (label, key) in toolbar_actions(app) {
        if x + 12 >= bar.right() {
            break;
        }
        put(buf, x, area.y, label, plain);
        x += cell_width(label) + 1;
        put(buf, x, area.y, key, key_style);
        x += cell_width(key) + 2;
        put(
            buf,
            x,
            area.y,
            "│",
            Style::default().fg(Color::Rgb(148, 163, 184)).bg(BAR_BG),
        );
        x += 2;
    }
    let right = "Menu m   Help ?   Quit q";
    let w = cell_width(right);
    if bar.right() > w {
        put(buf, bar.right().saturating_sub(w + 1), area.y, right, plain);
    }
}

fn mode_extra_text(app: &App) -> String {
    match app.mode {
        GameMode::Klondike => {
            let redeals = match app.klondike.redeals_remaining() {
                Some(n) => format!(" · Passes {n}"),
                None => String::new(),
            };
            let draw = if app.draw_mode == crate::klondike::DrawMode::Draw1 {
                "Draw 1"
            } else {
                "Draw 3"
            };
            format!(
                "Stock {} · Waste {} · {draw}{redeals}",
                app.klondike.stock.len(),
                app.klondike.waste.len()
            )
        }
        GameMode::FreeCell => format!(
            "Cells used {}/{} · foundations on contrary-color runs",
            app.freecell.free_count(),
            app.freecell.num_cells
        ),
        GameMode::SpiderMini => format!(
            "Stock deals {} · Completed {}/8 · {}",
            app.spider.stock_deals_remaining(),
            app.spider.completed_sequences,
            app.spider.suits.label()
        ),
    }
}

fn render_info_strip(buf: &mut Buffer, area: Rect, app: &App) {
    let y = area.y;
    put(
        buf,
        area.x + 2,
        y,
        &mode_extra_text(app),
        Style::default().fg(DIM_ON_FELT).bg(FELT),
    );
    let undo = format!("Undo {}", app.undo_depth());
    let w = cell_width(&undo);
    put(
        buf,
        area.right().saturating_sub(w + 2),
        y,
        &undo,
        Style::default().fg(DIM_ON_FELT).bg(FELT),
    );
}

fn render_message_strip(buf: &mut Buffer, area: Rect, app: &App) {
    let y = area.y;
    let dot = Style::default()
        .fg(TARGET_RING)
        .bg(FELT)
        .add_modifier(Modifier::BOLD);
    put(buf, area.x + 1, y, "●", dot);
    let max = area.width.saturating_sub(4) as usize;
    let msg = truncate_cells(&app.status_message, max);
    put(
        buf,
        area.x + 3,
        y,
        &msg,
        Style::default().fg(MSG_FG).bg(FELT),
    );
}

/// Bottom status bar: cursor context left, key hints right.
fn render_status_bar(buf: &mut Buffer, area: Rect, app: &App) {
    let bar = Rect::new(area.x, area.y, area.width, 1);
    fill(buf, bar, Style::default().bg(TITLE_BG));
    let style = Style::default().fg(Color::White).bg(TITLE_BG);
    let dim = Style::default().fg(DIM_ON_FELT).bg(TITLE_BG);
    let left = cursor_context(app);
    put(
        buf,
        area.x + 1,
        area.y,
        &truncate_cells(&left, area.width.saturating_sub(30) as usize),
        style,
    );
    let right = "Space select · Tab piles · 1–0 jump · ? help";
    let w = cell_width(right);
    if bar.width > w + 2 {
        put(buf, bar.right().saturating_sub(w + 1), area.y, right, dim);
    }
}

fn cursor_context(app: &App) -> String {
    let holding = match app.selected {
        Some(sel) => format!(" · holding {} — pick a green target", sel.count),
        None => String::new(),
    };
    let place = match app.cursor.area {
        CursorArea::Stock => match app.mode {
            GameMode::Klondike => format!("Stock · {} left", app.klondike.stock.len()),
            GameMode::SpiderMini => {
                format!("Stock · {} deals left", app.spider.stock_deals_remaining())
            }
            GameMode::FreeCell => "Stock · none in FreeCell".to_string(),
        },
        CursorArea::Waste => match app.klondike.waste_top() {
            Some(c) => format!("Waste · {}", c.render()),
            None => "Waste · empty".to_string(),
        },
        CursorArea::Foundation => {
            let top = match app.mode {
                GameMode::Klondike => app
                    .klondike
                    .foundation_top(app.cursor.index)
                    .map(|c| c.render()),
                GameMode::FreeCell => app
                    .freecell
                    .foundation_top(app.cursor.index)
                    .map(|c| c.render()),
                GameMode::SpiderMini => None,
            };
            format!(
                "Foundation {} · {}",
                app.cursor.index + 1,
                top.unwrap_or_else(|| "empty".to_string())
            )
        }
        CursorArea::FreeCell => {
            let card = app.freecell.cell_card(app.cursor.index);
            format!(
                "Cell {} · {}",
                app.cursor.index + 1,
                card.map(|c| c.render())
                    .unwrap_or_else(|| "empty".to_string())
            )
        }
        CursorArea::Tableau => {
            let (len, top) = match app.mode {
                GameMode::Klondike => {
                    let p = &app.klondike.tableau[app.cursor.index];
                    (p.len(), p.last().map(|c| c.render()))
                }
                GameMode::FreeCell => {
                    let p = &app.freecell.tableau[app.cursor.index];
                    (p.len(), p.last().map(|c| c.render()))
                }
                GameMode::SpiderMini => {
                    let p = &app.spider.tableau[app.cursor.index];
                    (p.len(), p.last().map(|c| c.render()))
                }
            };
            format!(
                "Tableau {} · {len} cards · {}{holding}",
                app.cursor.index + 1,
                top.unwrap_or_else(|| "empty".to_string())
            )
        }
    };
    place
}

// ---- game screen ----

fn render_game(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.height < 12 || area.width < 40 {
        render_too_small(frame, area);
        return;
    }
    let buf = frame.buffer_mut();
    render_title_bar(buf, Rect::new(area.x, area.y, area.width, 1), app);
    render_toolbar(buf, Rect::new(area.x, area.y + 1, area.width, 1), app);
    render_info_strip(buf, Rect::new(area.x, area.y + 2, area.width, 1), app);

    // Computed once per frame rather than once per pile widget.
    let targets = app.legal_targets();

    let top_y = area.y + 3;
    let top_h = CARD_H + 1;
    let tab_y = top_y + top_h;
    let msg_y = area.bottom().saturating_sub(2);
    let status_y = area.bottom().saturating_sub(1);

    match app.mode {
        GameMode::Klondike => render_klondike_top(
            buf,
            app,
            &targets,
            Rect::new(area.x, top_y, area.width, top_h),
        ),
        GameMode::FreeCell => render_freecell_top(
            buf,
            app,
            &targets,
            Rect::new(area.x, top_y, area.width, top_h),
        ),
        GameMode::SpiderMini => render_spider_top(
            buf,
            app,
            &targets,
            Rect::new(area.x, top_y, area.width, top_h),
        ),
    }

    let tab_h = msg_y.saturating_sub(tab_y + 1);
    if tab_h >= 3 {
        match app.mode {
            GameMode::Klondike => render_fans(
                buf,
                app,
                &targets,
                Rect::new(area.x, tab_y, area.width, tab_h),
                &app.klondike.tableau,
                9,
            ),
            GameMode::FreeCell => render_fans(
                buf,
                app,
                &targets,
                Rect::new(area.x, tab_y, area.width, tab_h),
                &app.freecell.tableau,
                9,
            ),
            GameMode::SpiderMini => render_fans(
                buf,
                app,
                &targets,
                Rect::new(area.x, tab_y, area.width, tab_h),
                &app.spider.tableau,
                7,
            ),
        }
    }

    render_message_strip(buf, Rect::new(area.x, msg_y, area.width, 1), app);
    render_status_bar(buf, Rect::new(area.x, status_y, area.width, 1), app);
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();
    fill(buf, area, Style::default().bg(TITLE_BG));
    put_centered(
        buf,
        area,
        area.y + area.height / 2,
        "Enlarge the window to play Solitaire (min 40×12).",
        Style::default().fg(Color::White).bg(TITLE_BG),
    );
}

/// Center `n` card widgets of width `w` in `area`, returning their x offsets.
fn center_slots(area: Rect, n: usize, w: u16) -> Vec<u16> {
    if n == 0 {
        return Vec::new();
    }
    let gap = if n > 1 {
        let natural = area.width.saturating_sub(n as u16 * w) / (n as u16 - 1);
        natural.clamp(1, 4)
    } else {
        0
    };
    let total = n as u16 * w + gap * (n as u16 - 1);
    let mut x0 = area.x + area.width.saturating_sub(total) / 2;
    // Keep the row on screen even in narrow windows.
    x0 = x0.min(area.right().saturating_sub(w));
    (0..n).map(|i| x0 + i as u16 * (w + gap)).collect()
}

// ---- top rows ----

fn render_klondike_top(buf: &mut Buffer, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let w: u16 = 9;
    let xs = center_slots(area, 6, w);
    if xs.len() < 6 {
        return;
    }
    // Stock.
    let stock_state = pile_state(app, targets, CursorArea::Stock, 0);
    if app.klondike.stock.is_empty() {
        let ghost = if app.klondike.waste.is_empty() {
            "·"
        } else {
            "↻"
        };
        paint_slot(buf, xs[0], area.y, w, ghost, ring_color(stock_state));
    } else {
        paint_card_back(buf, xs[0], area.y, w, ring_color(stock_state));
    }
    paint_caption(
        buf,
        xs[0],
        area.y + CARD_H,
        w,
        &format!("Stock {}", app.klondike.stock.len()),
        stock_state,
    );
    // Waste.
    let waste_state = pile_state(app, targets, CursorArea::Waste, 0);
    paint_pile_card(
        buf,
        xs[1],
        area.y,
        w,
        app.klondike.waste_top().as_ref(),
        "–",
        waste_state,
    );
    paint_caption(buf, xs[1], area.y + CARD_H, w, "Waste", waste_state);
    // Foundations.
    for f in 0..4 {
        let state = pile_state(app, targets, CursorArea::Foundation, f);
        paint_pile_card(
            buf,
            xs[2 + f],
            area.y,
            w,
            app.klondike.foundation_top(f).as_ref(),
            "A",
            state,
        );
        paint_caption(
            buf,
            xs[2 + f],
            area.y + CARD_H,
            w,
            &format!("F{}", f + 1),
            state,
        );
    }
}

fn render_freecell_top(buf: &mut Buffer, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let n = app.freecell.num_cells + 4;
    let w: u16 = if n > 7 { 7 } else { 9 };
    let xs = center_slots(area, n, w);
    if xs.len() < n {
        return;
    }
    for (c, &x) in xs.iter().enumerate().take(app.freecell.num_cells) {
        let state = pile_state(app, targets, CursorArea::FreeCell, c);
        paint_pile_card(
            buf,
            x,
            area.y,
            w,
            app.freecell.cell_card(c).as_ref(),
            "·",
            state,
        );
        paint_caption(buf, x, area.y + CARD_H, w, &format!("C{}", c + 1), state);
    }
    for f in 0..4 {
        let state = pile_state(app, targets, CursorArea::Foundation, f);
        paint_pile_card(
            buf,
            xs[app.freecell.num_cells + f],
            area.y,
            w,
            app.freecell.foundation_top(f).as_ref(),
            "A",
            state,
        );
        paint_caption(
            buf,
            xs[app.freecell.num_cells + f],
            area.y + CARD_H,
            w,
            &format!("F{}", f + 1),
            state,
        );
    }
}

fn render_spider_top(buf: &mut Buffer, app: &App, targets: &[(CursorArea, usize)], area: Rect) {
    let w: u16 = 9;
    let xs = center_slots(area, 3, w);
    if xs.len() < 3 {
        // Fall back to a single info line on narrow screens.
        put(
            buf,
            area.x + 2,
            area.y + 1,
            &mode_extra_text(app),
            Style::default().fg(DIM_ON_FELT).bg(FELT),
        );
        return;
    }
    // Stock deals remaining, shown as a card back with a count caption.
    let state = pile_state(app, targets, CursorArea::Stock, 0);
    if app.spider.stock_deals_remaining() == 0 {
        paint_slot(buf, xs[0], area.y, w, "·", ring_color(state));
    } else {
        paint_card_back(buf, xs[0], area.y, w, ring_color(state));
    }
    paint_caption(
        buf,
        xs[0],
        area.y + CARD_H,
        w,
        &format!("Stock ×{}", app.spider.stock_deals_remaining()),
        state,
    );
    // Completed sequences as a banked-run card.
    paint_slot(
        buf,
        xs[1],
        area.y,
        w,
        &format!("{}/8", app.spider.completed_sequences),
        SLOT_BORDER,
    );
    paint_caption(
        buf,
        xs[1],
        area.y + CARD_H,
        w,
        "Completed",
        PileState::Normal,
    );
    paint_slot(buf, xs[2], area.y, w, app.spider.suits.label(), SLOT_BORDER);
    paint_caption(buf, xs[2], area.y + CARD_H, w, "Suits", PileState::Normal);
}

// ---- tableau fans ----

/// Render tableau columns as overlapping desktop-style card fans.
fn render_fans(
    buf: &mut Buffer,
    app: &App,
    targets: &[(CursorArea, usize)],
    area: Rect,
    piles: &[Vec<Card>],
    card_w: u16,
) {
    let n = piles.len();
    let xs = center_slots(area, n, card_w);
    if xs.len() < n || area.height < 3 {
        return;
    }
    let header_y = area.y;
    let fan_y = area.y + 1;

    // Fan geometry: roomy overlapping widgets when the column fits (face-up
    // cards at stride 2 so rank strips stay visible, face-downs collapsed to
    // a 1-row edge); tall columns compress buried cards to 1-row rank
    // strips. Still-overflowing columns anchor to the pile top (the playable
    // end) and hide the buried cards under a count badge.

    for (i, pile) in piles.iter().enumerate() {
        let x = xs[i];
        let col_state = pile_state(app, targets, CursorArea::Tableau, i);
        // Header: keyboard number for the column, highlighted on focus.
        let key = if piles.len() == 10 {
            if i == 9 {
                "0".to_string()
            } else {
                format!("{}", i + 1)
            }
        } else {
            format!("{}", i + 1)
        };
        paint_caption(buf, x, header_y, card_w, &key, col_state);

        if pile.is_empty() {
            paint_slot(buf, x, fan_y, card_w, "K", ring_color(col_state));
            continue;
        }
        let bottom = area.bottom();
        let last = pile.len() - 1;
        let mut raw: Vec<u16> = Vec::with_capacity(pile.len());
        let compact: bool;
        if pile.len() == 1 {
            compact = false;
            raw.push(fan_y);
        } else {
            let downs = pile.iter().filter(|c| !c.face_up).count() as u16;
            let ups = pile.len() as u16 - downs;
            let roomy_end = fan_y + downs + 2 * ups.saturating_sub(1) + CARD_H;
            compact = roomy_end > bottom;
            if compact {
                for j in 0..pile.len() {
                    raw.push(fan_y + j as u16);
                }
            } else {
                let mut yy = fan_y;
                for card in pile {
                    raw.push(yy);
                    yy += if card.face_up { 2 } else { 1 };
                }
            }
        }
        let shift = raw[last].saturating_add(CARD_H).saturating_sub(bottom);
        // Cards paint top-to-bottom so later cards overlap earlier ones.
        let grabbed_count = match app.selected {
            Some(sel) if sel.area == CursorArea::Tableau && sel.index == i => sel.count,
            _ => 0,
        };
        let mut hidden_top = 0;
        for (j, (card, &cy)) in pile.iter().zip(raw.iter()).enumerate() {
            let dy = cy.saturating_sub(shift);
            if dy < fan_y {
                hidden_top += 1;
                continue;
            }
            let in_grab = j >= pile.len().saturating_sub(grabbed_count);
            let ring = if in_grab {
                GRABBED_RING
            } else {
                ring_color(col_state)
            };
            if compact && j < last {
                paint_strip(buf, x, dy, card_w, card, ring);
            } else if card.face_up {
                paint_card_face(buf, x, dy, card_w, card, ring);
            } else {
                paint_card_back(buf, x, dy, card_w, ring);
            }
        }
        if hidden_top > 0 {
            let label = format!("▲{hidden_top}");
            let style = Style::default().fg(Color::Black).bg(Color::Yellow);
            let clipped = truncate_cells(&label, card_w as usize);
            let pad = card_w.saturating_sub(cell_width(&clipped));
            put(buf, x, fan_y, &" ".repeat(card_w as usize), style);
            put(buf, x + pad / 2, fan_y, &clipped, style);
        }
    }
}

// ---- menu (desktop new-game window) ----

fn render_menu(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    render_title_bar(buf, Rect::new(area.x, area.y, area.width, 1), app);
    render_status_bar(
        buf,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        app,
    );

    let body = dialog_frame(frame, 64, 19, "Solitaire — New game", "m close");
    let Some(body) = body else { return };
    let buf = frame.buffer_mut();

    let games = GameMode::all();
    let blurbs = [
        "7 piles · stock + waste · 4 foundations",
        "8 piles · free cells · every card visible",
        "10 piles · deal rows · clear K→A runs",
    ];
    let mut y = body.y;
    for (i, mode) in games.iter().enumerate() {
        let active = i == app.menu_index;
        let row = Rect::new(body.x, y, body.width, 2);
        if active {
            fill(buf, row, Style::default().bg(SELECT_BG));
        }
        let marker = if active { "▸" } else { " " };
        let marker_style = Style::default()
            .fg(INK)
            .bg(if active { SELECT_BG } else { DIALOG_BG })
            .add_modifier(Modifier::BOLD);
        put(buf, body.x + 2, y, marker, marker_style);
        put(
            buf,
            body.x + 4,
            y,
            &format!("{}  {}", i + 1, mode.title()),
            Style::default()
                .fg(INK)
                .bg(if active { SELECT_BG } else { DIALOG_BG })
                .add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            body.x + 4,
            y + 1,
            blurbs[i],
            Style::default().fg(Color::Rgb(71, 85, 105)).bg(if active {
                SELECT_BG
            } else {
                DIALOG_BG
            }),
        );
        y += 2;
    }

    y += 1;
    put(
        buf,
        body.x + 2,
        y,
        "Difficulty",
        Style::default()
            .fg(INK)
            .bg(DIALOG_BG)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    let diffs = ["Easy", "Normal", "Hard"];
    let mut x = body.x + 4;
    for (i, d) in diffs.iter().enumerate() {
        let active = i == app.menu_diff_index;
        let radio = if active { "(●)" } else { "(○)" };
        let style = if active {
            Style::default()
                .fg(INK)
                .bg(DIALOG_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(71, 85, 105)).bg(DIALOG_BG)
        };
        put(buf, x, y, radio, style);
        x += cell_width(radio) + 1;
        put(buf, x, y, d, style);
        x += cell_width(d) + 4;
    }
    y += 2;
    let note_style = Style::default().fg(Color::Rgb(71, 85, 105)).bg(DIALOG_BG);
    put(
        buf,
        body.x + 2,
        y,
        "FreeCell cells: Easy 5 · Normal 4 · Hard 3",
        note_style,
    );
    put(
        buf,
        body.x + 2,
        y + 1,
        "Spider suits: Easy 1 · Normal 2 · Hard 4",
        note_style,
    );
    // Start button.
    let btn = "[  Start · Enter  ]";
    let btn_w = cell_width(btn);
    let btn_x = body.x + body.width.saturating_sub(btn_w) / 2;
    let btn_y = body.bottom().saturating_sub(2);
    put(
        buf,
        btn_x,
        btn_y,
        btn,
        Style::default()
            .fg(Color::White)
            .bg(TITLE_BG)
            .add_modifier(Modifier::BOLD),
    );
    put_centered(
        buf,
        Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
        body.bottom().saturating_sub(1),
        "↑↓ game · ←→ difficulty · Enter start · q back",
        Style::default().fg(Color::Rgb(71, 85, 105)).bg(DIALOG_BG),
    );
    if !app.status_message.is_empty() {
        put_centered(
            buf,
            Rect::new(body.x, body.bottom(), body.width, 1),
            body.bottom(),
            &app.status_message,
            Style::default().fg(MSG_FG).bg(FELT),
        );
    }
}

// ---- overlays ----

/// Centered modal: drop shadow, light body, dark title strip.
/// Returns the interior body rect, or None when the screen is too small.
fn dialog_frame(frame: &mut Frame, w: u16, h: u16, title: &str, extra: &str) -> Option<Rect> {
    let area = frame.area();
    if area.width < w + 2 || area.height < h + 1 {
        return None;
    }
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let outer = Rect::new(x, y, w, h);
    frame.render_widget(Clear, Rect::new(x, y, w + 1, h + 1));
    let buf = frame.buffer_mut();
    // Drop shadow.
    fill(
        buf,
        Rect::new(x + 1, y + 1, w, h),
        Style::default().bg(Color::Black),
    );
    // Body.
    fill(buf, outer, Style::default().bg(DIALOG_BG));
    // Title strip.
    let strip = Rect::new(x, y, w, 1);
    fill(buf, strip, Style::default().bg(TITLE_BG));
    put(
        buf,
        x + 2,
        y,
        title,
        Style::default()
            .fg(Color::White)
            .bg(TITLE_BG)
            .add_modifier(Modifier::BOLD),
    );
    if !extra.is_empty() {
        let wdt = cell_width(extra);
        put(
            buf,
            outer.right().saturating_sub(wdt + 2),
            y,
            extra,
            Style::default().fg(DIM_ON_FELT).bg(TITLE_BG),
        );
    }
    Some(Rect::new(x + 2, y + 2, w - 4, h.saturating_sub(4)))
}

fn render_help_overlay(frame: &mut Frame) {
    let rows: [(&str, &str); 13] = [
        ("← → ↑ ↓ / hjkl", "move cursor between piles"),
        ("Tab / Shift-Tab", "cycle pile rows"),
        ("Space / Enter", "grab cards · place on target"),
        ("d", "draw (Klondike) · deal row (Spider)"),
        ("a", "auto-move safe cards to foundations"),
        ("u", "undo"),
        ("H", "hint"),
        ("1–8 · 9 · 0", "jump to tableau pile"),
        ("s w f c", "focus stock · waste · foundation · cells"),
        ("n / r", "new game · restart"),
        ("m", "menu · change game + difficulty"),
        ("?", "toggle this help"),
        ("q / Esc", "cancel grab · quit"),
    ];
    let Some(body) = dialog_frame(frame, 62, 20, "Help — Keyboard", "Esc close") else {
        return;
    };
    let buf = frame.buffer_mut();
    for (i, (key, desc)) in rows.iter().enumerate() {
        let y = body.y + i as u16;
        if y >= body.bottom() {
            break;
        }
        put(
            buf,
            body.x,
            y,
            key,
            Style::default()
                .fg(INK)
                .bg(DIALOG_BG)
                .add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            body.x + 22,
            y,
            desc,
            Style::default().fg(INK).bg(DIALOG_BG),
        );
    }
}

fn render_win_overlay(frame: &mut Frame, app: &App) {
    let Some(body) = dialog_frame(frame, 54, 9, "You win!", "") else {
        return;
    };
    let buf = frame.buffer_mut();
    put_centered(
        buf,
        Rect::new(body.x, body.y, body.width, 1),
        body.y,
        &format!(
            "{} · {}",
            app.mode.title(),
            difficulty_label(app.difficulty)
        ),
        Style::default()
            .fg(INK)
            .bg(DIALOG_BG)
            .add_modifier(Modifier::BOLD),
    );
    put_centered(
        buf,
        Rect::new(body.x, body.y + 1, body.width, 1),
        body.y + 1,
        &format!(
            "Score {} in {} moves ({})",
            app.score(),
            app.moves(),
            fmt_time(app.elapsed_secs())
        ),
        Style::default().fg(Color::Rgb(71, 85, 105)).bg(DIALOG_BG),
    );
    put_centered(
        buf,
        Rect::new(body.x, body.y + 3, body.width, 1),
        body.y + 3,
        "[ n new game ]   [ m menu ]   [ q quit ]",
        Style::default()
            .fg(Color::White)
            .bg(TITLE_BG)
            .add_modifier(Modifier::BOLD),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::klondike::{Difficulty, DrawMode};
    use ratatui::{backend::TestBackend, Terminal};

    fn screen_text(app: &App, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal.draw(|f| render(f, app)).expect("render succeeds");
        let buffer = terminal.backend().buffer().clone();
        let mut rows = Vec::new();
        for y in 0..height {
            let mut row = String::new();
            for x in 0..width {
                row.push_str(buffer[(x, y)].symbol());
            }
            rows.push(row);
        }
        rows
    }

    #[test]
    fn fresh_app_boots_into_start_menu() {
        let app = App::new(DrawMode::Draw3, Difficulty::Normal);
        assert_eq!(app.screen, Screen::Menu);
        let rows = screen_text(&app, 80, 24);
        let body = rows.join("\n");
        assert!(body.contains("New game"), "start menu:\n{body}");
        assert!(body.contains("Difficulty"), "difficulty picker:\n{body}");
    }

    #[test]
    fn game_chrome_has_title_toolbar_and_status_bars() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.screen = Screen::Game;
        let rows = screen_text(&app, 80, 24);
        assert!(rows[0].contains("Solitaire"), "title bar:\n{}", rows[0]);
        assert!(rows[0].contains("Klondike"), "doc title:\n{}", rows[0]);
        assert!(rows[1].contains("Undo"), "toolbar:\n{}", rows[1]);
        assert!(rows[1].contains("Quit"), "toolbar right:\n{}", rows[1]);
        assert!(
            rows[23].contains("Space select"),
            "status bar:\n{}",
            rows[23]
        );
        // Card widgets use rounded corners; no boxed-pile titles remain.
        let body = rows.join("\n");
        assert!(body.contains("╭"), "expected card widgets");
        assert!(!body.contains("S stock"), "no legacy pile boxes");
    }

    #[test]
    fn felt_background_covers_the_table() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.screen = Screen::Game;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal.draw(|f| render(f, &app)).expect("render");
        let buffer = terminal.backend().buffer().clone();
        assert_eq!(buffer[(0, 5)].bg, FELT);
    }

    #[test]
    fn all_modes_render_without_panic() {
        for mode in GameMode::all() {
            let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
            app.screen = Screen::Game;
            app.switch_mode(mode);
            app.ensure_cursor_valid();
            let rows = screen_text(&app, 80, 24);
            assert!(rows[0].contains(mode.title()));
        }
    }

    #[test]
    fn menu_help_and_tiny_windows_render() {
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.open_menu();
        let rows = screen_text(&app, 80, 24);
        assert!(rows.join("\n").contains("New game"));
        app.screen = Screen::Game;
        app.show_help = true;
        let rows = screen_text(&app, 80, 24);
        assert!(rows.join("\n").contains("Keyboard"));
        // Tiny window shows the resize note instead of panicking.
        let mut app = App::new(DrawMode::Draw3, Difficulty::Normal);
        app.screen = Screen::Game;
        let rows = screen_text(&app, 30, 10);
        assert!(rows.join("\n").contains("Enlarge"));
    }
}
