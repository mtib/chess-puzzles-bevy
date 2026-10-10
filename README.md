# chess-puzzles-bevy

Native twin of the Omarchy `mtib.chesspuzzles` panel (Quickshell/QML), built
with Rust + Bevy — for comparing the two implementations.

Feature parity with the QML panel:

- Lichess puzzles served by the same `pick.sh` from the same shared local
  cache (`~/.local/state/omarchy/chesspuzzles/cache.jsonl`), targeted at a
  600–2500 ELO slider (step 50).
- Click-click input, judged against the puzzle's solution list; opponent
  replies auto-play after 650ms; "Show solution" walks the line at 650ms a
  move; wrong attempts flash the target square; promotions accepted on the
  first four characters.
- Monochrome dark board painted on a CPU texture, mirroring the QML Canvas:
  dark background, dashed cell outlines, diagonal-hatched light squares,
  orange fill on the last move's squares, lighter orange selection, fading
  move arrows, shift+drag user arrows (cleared by clicking an empty square).
- Board flips so the side to move sits at the bottom.
- Solved count + target ELO persist (separate state file from the QML panel:
  `~/.local/state/omarchy/settings/chesspuzzles-bevy.json`).
- Black knight as the logo (window icon + header).

Run:

```sh
cargo run --release
```

Keyboard: `n` new puzzle, `s` show solution, `f` flip board.

Piece art: [caliente](https://github.com/avi-0/caliente) by avi-0
(converted from SVG to PNG in `assets/pieces/`).
