# Solitaire (TUI)

Play Klondike, FreeCell, and Spider-mini in the terminal. Built with
[Ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm).

## Run

Requires a Rust toolchain (stable, 1.75+).

```sh
cargo run
cargo run -- --draw 1 --difficulty easy   # Klondike draw-1, easy mode
cargo run -- --help                       # all CLI options
```

The app opens on a start menu: pick a game (`↑↓` or `1`/`2`/`3`), pick a
difficulty (`←→`), `Enter` to deal. `m` returns to it mid-game.

CLI options:

| Flag | Values | Default | Effect |
| ---- | ------ | ------- | ------ |
| `--draw` | `1`, `3` | `3` | Klondike stock draw count |
| `--difficulty` | `easy`, `normal`, `hard` | `normal` | Starting difficulty for all games |

## Games

| Key (menu) | Game | Rules highlights |
| ---------- | ---- | ---------------- |
| `1` | Klondike | 7 tableau columns (alternating-color, descending), 4 suit-ascending foundations, stock/waste with redeals |
| `2` | FreeCell | 8 tableau columns, free cells + 4 foundations; empty-column supermoves |
| `3` | Spider-mini | 10 tableau columns, build same-suit K→A runs to bank all 8 sequences |

## Difficulty

| Difficulty | Klondike | FreeCell | Spider-mini |
| ---------- | -------- | -------- | ----------- |
| Easy | Draw-1, unlimited redeals | 5 free cells | 1 suit |
| Normal | Draw-3, unlimited redeals | 4 free cells | 2 suits |
| Hard | Draw-3, max 3 redeals then game over | 3 free cells | 4 suits |

## Controls

| Key | Action |
| --- | ------ |
| `←↓↑→` / `hjkl` | Move cursor |
| `Tab` / `Shift-Tab` | Cycle focus areas (stock, waste, foundations, cells, tableau) |
| `Space` / `Enter` | Grab / place cards |
| `d` | Draw from stock (Klondike) / deal a row (Spider) |
| `a` | Auto-move safe cards to foundations |
| `H` (shift-h) | Hint |
| `u` | Undo (up to 300 steps) |
| `s` / `w` / `f` / `c` / `t` | Jump to stock / waste / foundation / cells / tableau |
| `1`–`9`, `0` | Jump to tableau column 1–10 |
| `n` / `r` | New game / restart |
| `m` | Game/difficulty menu |
| `?` | Help overlay |
| `q` / `Esc` | Cancel selection, then quit |

## Screen layout (ASCII sketch)

The table mimics a desktop card app: window title bar, toolbar, green
felt, card widgets, and a status bar. Menu, help, and victory appear as
centered modal dialogs.

```text
 ● ● ●  Solitaire            — Klondike · Normal —    Score 125  Moves 34  03:12
 New n  │ Undo u  │ Draw d  │ Auto a  │ Hint H  │      Menu m   Help ?   Quit q
  Stock 24 · Waste 0 · Draw 3                                            Undo 5
   ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮    ╭───────╮
   │▓ ▓ ▓ ▓│    │ 9♥    │    │ A♠    │    │       │    │       │    │       │
   │▓ ▓ ▓ ▓│    │   ♥   │    │   ♠   │    │   A   │    │   A   │    │   A   │
   │▓ ▓ ▓ ▓│    │    9♥ │    │    A♠ │    │       │    │       │    │       │
   ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯    ╰───────╯
   Stock 24       Waste         F1           F2           F3           F4
      1          2          3          4          5          6          7
  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  │5♣     │  │K♠     │  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮  ╭───────╮
  │   ♣   │  │   ♠   │  │4♥     │  │…      │  │…      │  │…      │  │…      │
  │     5♣│  │     K♠│  │   ♥   │  │…      │  │…      │  │…      │  │…      │
  ╰───────╯  ╰───────╯  │     4♥│  ╰───────╯  ╰───────╯  ╰───────╯  ╰───────╯
                       ╰───────╯
 ● Hint: move 5♣ onto 6♦ (T4)
 Tableau 4 · 3 cards · 4♥              Space select · Tab piles · 1–0 jump · ? help
```

Tableau columns are overlapping fans: face-down cards collapse to a thin
edge, tall columns compress buried cards to one-row rank strips, and
overflowing columns anchor to the playable top end with a `▲N` badge.

Cursor is white, grabbed cards are yellow, legal drop targets are green.
Red suits render red, black suits dark, face-down cards show a blue back.

## Scoring (Klondike)

+10 foundation, +5 waste→tableau / card flip, +3 tableau move,
−20 redeal (floored at 0), +500 win bonus. Foundation→tableau costs 15.

## Develop

```sh
cargo fmt        # format
cargo clippy     # lints (dead-code warnings for unused helpers are expected)
cargo test       # 35 unit tests: rules, scoring, winnable-deal solver, app/undo
cargo build      # debug binary at ./target/debug/solitaire
```
