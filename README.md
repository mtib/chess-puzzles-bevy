# chess-puzzles-bevy

Native chess puzzle trainer built with Rust + [Bevy](https://bevyengine.org)
0.20: solve [Lichess](https://lichess.org) puzzles at a target ELO on a
monochrome, terminal-styled board.

![Chess Puzzles: monochrome terminal-styled board, coral move highlights](docs/screenshot.png)

## Features

- **ELO slider** (600–2500, step 50): puzzles are served from a local cache,
  picking the one closest to your target rating while avoiding the ~80 most
  recently served.
- **Offline cache** seeded from the
  [Lichess puzzle database](https://database.lichess.org/) — popular puzzles
  bucketed across the whole rating range. `pick.sh` falls back to a live
  Lichess fetch and appends to the cache, so it grows on its own.
- **Click-click input**, judged against the puzzle's solution list (no move
  generator — the solution is the only judge). Opponent replies auto-play
  after 650ms; "Show solution" walks the line at 650ms a move; wrong attempts
  flash the target square; promotions accepted on the first four characters.
- **Monochrome dark board**, CPU-painted to a texture (HiDPI-aware — painted
  at the display's scale factor): pure black background, diagonal-hatched
  light squares, dashed cell outlines, coral `#FF5232` for moved/selected
  squares and move arrows, monochrome user-drawn arrows
  (**shift+drag**), cleared by clicking an empty square.
- **Responsive**: the board, cursor mapping, and slider all derive from the
  live window size — any window size just works.
- Board flips so the side to move sits at the bottom. Keyboard: `n` new
  puzzle, `s` show solution, `f` flip board.

## Run

```sh
cargo run --release
```

Optional assets: piece art ships in `assets/pieces/` (see credits). A font is
expected at `assets/fonts/`; any TTF works — e.g. JetBrains Mono:

```sh
mkdir -p assets/fonts
cp /usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf assets/fonts/
```

The puzzle cache and state live under `~/.local/state/omarchy/`
(`chesspuzzles/cache.jsonl`, `settings/chesspuzzles-bevy.json`). Point
`CHESS_PUZZLES_PICK` at your own picker script to change where puzzles come
from; it just needs to print one JSON record per invocation:

```json
{"id":"…","fen":"…","rating":2205,"themes":[…],"solution":["h5h7",…]}
```

## Layout

```
src/main.rs    Bevy app: window, UI, input, timers, board placement
src/state.rs   puzzle state machine, persistence, puzzle fetching
src/model.rs   FEN parsing + UCI move application (castling, EP, promotion)
src/paint.rs   CPU board painter (hatch, dashes, arrows, piece blitting)
scripts/pick.sh  serve one puzzle near a target ELO from the cache
scripts/seed.py  one-off cache seeding from the Lichess puzzle database
```

## Credits

- Pieces: [caliente](https://github.com/avi-0/caliente) by avi-0, converted
  to PNG. No legal-move engine: the puzzle solutions come from Lichess's
  public puzzle database (CC0).
- The Quickshell/QML twin of this app — the same trainer as an Omarchy bar
  panel:
  **[mtib/chess-puzzles-qml](https://github.com/mtib/chess-puzzles-qml)**.

## License

MIT
