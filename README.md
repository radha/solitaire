# Solitaire (TUI)

Play Klondike, FreeCell, and Spider in the terminal. Built with
[Ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm).

## Run

Requires a Rust toolchain, 1.88 or newer.

```sh
cargo run --release
cargo run --release -- --difficulty easy        # Easy: draw-1, solvable deals
cargo run --release -- --draw 1 --deal 12345    # replay deal #12345, draw-1
cargo run --release -- --help                   # all CLI options
```

The app opens on a start menu: pick a game (`↑↓` or `1`/`2`/`3`), pick a
difficulty (`←→`), optionally toggle the Klondike draw count (`d`), then
`Enter`. If the chosen game is already in progress with the same settings,
`Enter` resumes it; otherwise it deals a new one (asking first if that would
discard progress). `m` returns to the menu mid-game.

CLI options:

| Flag | Values | Default | Effect |
| ---- | ------ | ------- | ------ |
| `--difficulty` | `easy`, `normal`, `hard` | `normal` | Starting difficulty for all games |
| `--draw` | `1`, `3` | `1` on Easy, `3` otherwise | Klondike stock draw count |
| `--deal` | any `u32` | random | Deal number of the first game |

Every game shows its deal number (`Deal #…`). Deals are reproducible:
`r` restarts the same deal and `--deal N` replays one.

## Games

| Key (menu) | Game | Rules highlights |
| ---------- | ---- | ---------------- |
| `1` | Klondike | 7 tableau columns (alternating colors, descending), 4 foundations, stock/waste with redeals |
| `2` | FreeCell | 8 tableau columns, free cells + 4 foundations; multi-card moves limited by free cells and empty columns |
| `3` | Spider | 10 tableau columns, 104 cards; build same-suit K→A runs to clear all 8 |

## Difficulty

Each game keeps its own difficulty, so switching games never changes the
rules of a game in progress.

| Difficulty | Klondike | FreeCell | Spider |
| ---------- | -------- | -------- | ------ |
| Easy | Draw-1, unlimited redeals, deal verified solvable | 5 free cells | 1 suit |
| Normal | Draw-3, unlimited redeals | 4 free cells | 2 suits |
| Hard | Draw-3, 3 redeals | 3 free cells | 4 suits |

## Controls

| Key | Action |
| --- | ------ |
| `←→` / `hl` | Move along a row (crossing stock, waste, cells, foundations) |
| `↑↓` / `kj` | Jump between the top row and the tableau |
| `Tab` / `Shift-Tab` | Cycle pile areas |
| `Space` / `Enter` | Grab / place cards (draws when on the stock) |
| `+` / `-` | Hold more / fewer cards of the grabbed run |
| `F` (shift-f) | Send the cursor pile's top card to a foundation |
| `d` | Draw from stock (Klondike) / deal a row (Spider) |
| `a` | Auto-move safe cards to foundations; finishes the game once every card is in order |
| `H` (shift-h) | Hint |
| `u` | Undo (up to 300 steps) |
| `s` / `w` / `f` / `c` / `t` | Jump to stock / waste / foundation / cells / tableau |
| `1`–`9`, `0` | Jump to tableau column 1–10 |
| `n` / `r` | New deal / restart this deal |
| `m` | Game/difficulty menu |
| `?` | Help overlay |
| `q` / `Esc` | Cancel selection, then quit |
| `Ctrl-C` | Quit immediately |

Mouse: click a pile to grab, click a target to place. Clicking a specific
card in a column grabs the run from that card down; clicking it again lets
go. Right-click sends a card to the foundations. The toolbar, menu and
dialog buttons are clickable.

New deal, restart and quit ask for confirmation only when a game is in
progress.

## Screen layout (ASCII sketch)

```text
 ● ● ●  Solitaire            — Klondike · Normal —    Score 15   Moves 3   00:42
 New n │ Restart r │ Undo u │ Draw d │ Auto a │     Menu m   Help ?   Quit q
  Deal #7 · Stock 15 · Waste 9 · Draw 3                                 Undo 3
   ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮
   │▓ ▓ ▓ ▓│    │2♣     │    │       │    │       │    │       │    │       │
   │▓ ▓ ▓ ▓│    │   ♣   │    │   A   │    │   A   │    │   A   │    │   A   │
   │▓ ▓ ▓ ▓│    │     2♣│    │       │    │       │    │       │    │       │
   ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯
   Stock 15       Waste         F1           F2           F3           F4
      1          2          3          4          5          6          7
  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  │8♥     │  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  │   ♥   │  │A♠     │  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  │     8♥│  │   ♠   │  │3♦     │  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  ╰───────╯  │     A♠│  │   ♦   │  │9♣     │  ╭───────╮  ╭───────╮  ╭───────╮
             ╰───────╯  │     3♦│  │   ♣   │  │5♠     │  ╭───────╮  ╭───────╮
 ● Move the top card of tableau pile 2 to a foundation.
 Tableau 7 · 7 cards · 4♠                                  Space select · ? help
```

Tableau columns are overlapping fans: face-down cards collapse to a thin
edge, tall columns compress buried cards to one-row rank strips, and
overflowing columns anchor to the playable top end with a `▲N` badge.

The cursor is white, grabbed cards are yellow, and legal drop targets are
green (including piles that take only part of a grabbed run). When no
useful move is left, the message line turns into a red banner with undo /
restart / new-deal buttons.

The minimum window is 69×18 for Klondike and 79×18 for FreeCell and Spider.
Colors use 24-bit RGB when the terminal advertises it (`COLORTERM=truecolor`)
and fall back to the 256-color palette otherwise.

## Hints

Hints only suggest moves that make progress: sending a card up, exposing a
face-down card, emptying a column for a waiting King, freeing a cell, or
building a longer run. Following hints therefore never loops, and when no
hint is left the game reports that no useful moves remain.

## Scoring

- **Klondike** (Windows standard): +10 to a foundation, +5 waste→tableau,
  +5 per card turned face up, −15 foundation→tableau, −20 per redeal
  (floored at 0), +500 win bonus. Tableau moves score nothing.
- **FreeCell**: +10 per card to a foundation, −10 when one comes back down,
  +500 win bonus.
- **Spider** (Windows style): start at 500, −1 per move or deal, +100 per
  completed run.

## Develop

```sh
cargo fmt        # format
cargo clippy --all-targets -- -W clippy::pedantic   # lints (clean)
cargo test       # rules, scoring, hints, solver, app flows, rendering
cargo build      # debug binary at ./target/debug/solitaire
```

Source layout: `cards.rs` (cards, seeded shuffle), `rules.rs` (difficulty,
shared rules, the `Solitaire` trait), `klondike.rs` / `freecell.rs` /
`spider.rs` (game rules, hints, Klondike solver), `app.rs` (sessions, menu,
cursor, mouse, undo), `ui.rs` (rendering, hit regions), `main.rs` (CLI,
event loop, key bindings).
