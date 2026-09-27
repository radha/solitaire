//! Solitaire TUI — binary entry point.

mod app;
mod cards;
mod freecell;
mod klondike;
mod spider;
mod ui;

use std::io;
use std::time::Duration;

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, Dir, GameMode, Screen};
use klondike::{Difficulty, DrawMode};

/// TUI Solitaire: Klondike, FreeCell, Spider-mini.
#[derive(Parser, Debug)]
#[command(name = "solitaire", version, about = "Play solitaire in the terminal")]
struct Cli {
    /// Draw mode for Klondike: 1 or 3.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8))]
    draw: u8,

    /// Difficulty: easy, normal, hard.
    #[arg(long, default_value = "normal")]
    difficulty: String,
}

fn parse_difficulty(s: &str) -> Difficulty {
    match s.to_lowercase().as_str() {
        "easy" => Difficulty::Easy,
        "hard" => Difficulty::Hard,
        _ => Difficulty::Normal,
    }
}

fn main() -> AnyhowResult {
    let cli = Cli::parse();
    let draw_mode = if cli.draw == 1 {
        DrawMode::Draw1
    } else {
        DrawMode::Draw3
    };
    let difficulty = parse_difficulty(&cli.difficulty);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(draw_mode, difficulty);
    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("error: {e}");
    }
    Ok(())
}

type AnyhowResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> AnyhowResult {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, app))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                handle_key(app, key.code, key.modifiers);
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // Menu screen has its own bindings.
    if app.screen == Screen::Menu {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if app.show_help {
                    app.show_help = false;
                } else {
                    app.screen = Screen::Game;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => app.menu_move(Dir::Up),
            KeyCode::Down | KeyCode::Char('j') => app.menu_move(Dir::Down),
            KeyCode::Left | KeyCode::Char('h') => app.menu_move(Dir::Left),
            KeyCode::Right | KeyCode::Char('l') => app.menu_move(Dir::Right),
            KeyCode::Enter | KeyCode::Char(' ') => app.menu_confirm(),
            KeyCode::Char('?') => app.toggle_help(),
            KeyCode::Char('m') => app.screen = Screen::Game,
            KeyCode::Char('1') => {
                app.menu_index = 0;
                app.menu_confirm();
            }
            KeyCode::Char('2') => {
                app.menu_index = 1;
                app.menu_confirm();
            }
            KeyCode::Char('3') => {
                app.menu_index = 2;
                app.menu_confirm();
            }
            _ => {}
        }
        return;
    }

    // Game screen.
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.show_help {
                app.show_help = false;
            } else if app.selected.is_some() {
                app.cancel_selection();
            } else {
                app.quit();
            }
        }
        KeyCode::Char('m') => app.open_menu(),
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor(Dir::Left),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor(Dir::Right),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(Dir::Up),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(Dir::Down),
        KeyCode::Tab => {
            if mods.contains(KeyModifiers::SHIFT) {
                app.tab_prev();
            } else {
                app.tab_next();
            }
        }
        KeyCode::BackTab => app.tab_prev(),
        KeyCode::Char(' ') | KeyCode::Enter => app.select_or_place(),
        KeyCode::Char('d') => app.draw(),
        KeyCode::Char('a') => app.auto_foundation(),
        KeyCode::Char('u') => app.undo(),
        // 'h' is cursor-left, so hint lives on shift-H.
        KeyCode::Char('H') => app.hint(),
        KeyCode::Char('n') => app.new_game(),
        KeyCode::Char('r') => app.restart(),
        KeyCode::Char('s') => app.focus_stock(),
        KeyCode::Char('w') => app.focus_waste(),
        KeyCode::Char('f') => app.focus_foundation(),
        KeyCode::Char('c') => app.focus_cells(),
        KeyCode::Char('t') => app.jump_to_tableau(0),
        KeyCode::Char('1') => app.jump_to_tableau(0),
        KeyCode::Char('2') => app.jump_to_tableau(1),
        KeyCode::Char('3') => app.jump_to_tableau(2),
        KeyCode::Char('4') => app.jump_to_tableau(3),
        KeyCode::Char('5') => app.jump_to_tableau(4),
        KeyCode::Char('6') => app.jump_to_tableau(5),
        KeyCode::Char('7') => app.jump_to_tableau(6),
        KeyCode::Char('8') => app.jump_to_tableau(7),
        KeyCode::Char('9') => app.jump_to_tableau(8),
        KeyCode::Char('0') => app.jump_to_tableau(9),
        _ => {}
    }
    let _ = GameMode::Klondike; // keep import meaningful in all builds
}
