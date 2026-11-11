# Notes for working on timechopper

The README is deliberately short (disclaimer, one-line summary, screenshots,
how to run it). Keep it that way: put behaviour and design in
`docs/design.md`, code-level detail in module docs (`//!`), and development
notes here.

## Where things are

- `docs/design.md`: what the live window shows, markers, parts of a task,
  learning from earlier days, the focus log format, how the guess is made,
  command-line corrections, the `Journal` API, known limits.
- `src/main.rs`: command-line parsing; `USAGE` lists every command and flag.
- `src/ui.rs`: the live window (egui/eframe). `src/live.rs`: clock, replay,
  log file. `src/win.rs`: the Windows watcher (`follow` feeds the window).
- `src/interpret/`: the interpretation. `mod.rs` gives the pipeline;
  `title.rs`, `structure.rs`, `journal.rs`, `history.rs` each document their
  step. `scenarios.rs` holds the synthetic logs used by `demo`, `--replay`
  and the tests.
- `tests/`: `interpret.rs`, `editing.rs`, `marks.rs`, `parts.rs`,
  `history.rs` show each behaviour on a synthetic log; `ui.rs` drives the
  window headlessly through the accessibility tree (egui_kittest).
- `ci/*.ps1` and `.github/workflows/windows.yml`: Windows smoke tests that
  drive Notepad, Calculator and the live window and record the screen.

## Commands (development only)

```
cargo test                                      # any platform
cargo fmt --check && cargo clippy --all-targets -- -D warnings   # CI runs both on Windows
cargo run -- demo                               # interpret the built-in synthetic logs
cargo run -- demo messy-day --log               # print one synthetic log
cargo run -- demo week                          # learning across a synthetic week
cargo run -- demo brief                         # parts of a task
cargo run -- summarize day.log --contents       # interpret a log file
cargo run -- ui --replay messy-day --speed 120  # the live window on any platform
cargo run -- --once                             # Windows: print the focused window
```

## Screenshots in the README

`docs/images/*.png` were taken on Linux under Xvfb from
`timechopper ui --replay messy-day --speed 120`, cropped to the 400×620
window (`import -window root`, then `convert -crop`). This needs
`libxkbcommon-x11-0` installed; `xdotool` can click a row open first.
