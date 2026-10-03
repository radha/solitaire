//! Solitaire TUI — binary entry point: CLI, terminal setup, event loop and
//! key bindings.

mod app;
mod cards;
mod freecell;
mod klondike;
mod rules;
mod spider;
mod ui;

use std::io;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

use app::{Action, App, CursorArea, Dir, GameMode, Screen};
use klondike::DrawMode;
use rules::Difficulty;

/// Play Klondike, FreeCell and Spider in the terminal.
#[derive(Parser, Debug)]
#[command(name = "solitaire", version, about)]
struct Cli {
    /// Klondike draw count [default: 1 on Easy, 3 otherwise].
    #[arg(long, value_enum)]
    draw: Option<DrawMode>,

    /// Starting difficulty for every game.
    #[arg(long, value_enum, default_value_t = Difficulty::Normal)]
    difficulty: Difficulty,

    /// Deal number for the first game (replays a specific layout).
    #[arg(long)]
    deal: Option<u32>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let mut app = App::new(cli.difficulty, cli.draw, cli.deal);
    app.truecolor = supports_truecolor();

    // `ratatui::init` enables raw mode + the alternate screen and installs a
    // panic hook that restores them; chain mouse-capture cleanup onto it.
    let mut terminal = ratatui::init();
    let restore_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        restore_hook(info);
    }));

    let result =
        execute!(io::stdout(), EnableMouseCapture).and_then(|()| run(&mut terminal, &mut app));
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

/// Whether the terminal advertises 24-bit color.
fn supports_truecolor() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
        || std::env::var_os("WT_SESSION").is_some()
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.should_quit {
        app.tick();
        let mut hits = Vec::new();
        terminal.draw(|f| hits = ui::render(f, app))?;
        app.hits = hits;
        // Sleep until input arrives or the clock reaches its next second.
        let to_next_second = 1000 - u64::from(app.elapsed().subsec_millis());
        if event::poll(Duration::from_millis(to_next_second.max(20)))? {
            handle_event(app, &event::read()?);
            // Drain anything else queued before redrawing.
            while !app.should_quit && event::poll(Duration::ZERO)? {
                handle_event(app, &event::read()?);
            }
        }
    }
    Ok(())
}

fn handle_event(app: &mut App, event: &Event) {
    match event {
        // Only presses: Windows also reports releases, which would double
        // every action.
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(app, *key),
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => app.click(mouse.column, mouse.row, false),
            MouseEventKind::Down(MouseButton::Right) => app.click(mouse.column, mouse.row, true),
            _ => {}
        },
        _ => {}
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(key.code, KeyCode::Char('c' | 'q')) {
            app.should_quit = true;
        }
        return;
    }
    // A pending destructive-action confirmation takes over all keys.
    if app.confirm.is_some() {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => app.confirm_pending(),
            _ => app.cancel_pending(),
        }
        return;
    }
    if app.show_help {
        app.show_help = false;
        return;
    }
    match app.screen {
        Screen::Menu => menu_key(app, key.code),
        Screen::Game => game_key(app, key.code),
    }
}

fn menu_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q' | 'm') | KeyCode::Esc => app.close_menu(),
        KeyCode::Up | KeyCode::Char('k') => app.menu_move(Dir::Up),
        KeyCode::Down | KeyCode::Char('j') => app.menu_move(Dir::Down),
        KeyCode::Left | KeyCode::Char('h') => app.menu_move(Dir::Left),
        KeyCode::Right | KeyCode::Char('l') => app.menu_move(Dir::Right),
        KeyCode::Enter | KeyCode::Char(' ') => app.menu_confirm(),
        KeyCode::Char('d') => app.menu_toggle_draw(),
        KeyCode::Char('?') => app.perform(Action::Help),
        KeyCode::Char(c @ '1'..='3') => {
            app.menu.game = GameMode::ALL[usize::from(c as u8 - b'1')];
        }
        _ => {}
    }
}

fn game_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.selected.is_some() {
                app.cancel_selection();
            } else {
                app.perform(Action::Quit);
            }
        }
        KeyCode::Char('m') => app.perform(Action::Menu),
        KeyCode::Char('?') => app.perform(Action::Help),
        KeyCode::Char('n') => app.perform(Action::New),
        KeyCode::Char('r') => app.perform(Action::Restart),
        KeyCode::Char('u') => app.perform(Action::Undo),
        KeyCode::Char('d') => app.perform(Action::Draw),
        KeyCode::Char('a') => app.perform(Action::Auto),
        // 'h' is cursor-left, so hint lives on shift-H.
        KeyCode::Char('H') => app.perform(Action::Hint),
        KeyCode::Char('F') => app.send_to_foundation(),
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor(Dir::Left),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor(Dir::Right),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(Dir::Up),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(Dir::Down),
        KeyCode::Tab => app.tab_next(),
        KeyCode::BackTab => app.tab_prev(),
        KeyCode::Char(' ') | KeyCode::Enter => app.select_or_place(),
        KeyCode::Char('+' | '=') => app.adjust_grab(true),
        KeyCode::Char('-' | '_') => app.adjust_grab(false),
        KeyCode::Char('s') => app.focus(CursorArea::Stock, 0),
        KeyCode::Char('w') => app.focus(CursorArea::Waste, 0),
        KeyCode::Char('f') => app.focus(CursorArea::Foundation, 0),
        KeyCode::Char('c') => app.focus(CursorArea::FreeCell, 0),
        KeyCode::Char('t') => app.focus(CursorArea::Tableau, 0),
        KeyCode::Char('0') => app.focus(CursorArea::Tableau, 9),
        KeyCode::Char(c @ '1'..='9') => {
            app.focus(CursorArea::Tableau, usize::from(c as u8 - b'1'));
        }
        _ => {}
    }
}
