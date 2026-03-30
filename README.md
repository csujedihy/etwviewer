# etwviewer

A terminal UI viewer for Windows ETW (Event Tracing for Windows) trace files (`.etl`).

![Platform](https://img.shields.io/badge/platform-Windows-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)

## Features

- Browse ETW events from `.etl` files in a fast, keyboard-driven TUI
- Full-text search with regex support and highlighted matches
- Filter events by provider or keyword
- Detail view with formatted event properties
- Visual selection and copy to clipboard
- Vim-style navigation (`j`/`k`, `Ctrl+U`/`Ctrl+D` for half-page scroll)
- Timestamp display modes (absolute / relative / UTC)
- Line wrap toggle, horizontal scrolling
- Position indicator scrollbar
- Parse timing display

## Requirements

- Windows (uses ETW APIs)
- Rust toolchain (`cargo`)

## Build

```powershell
cargo build --release
```

The binary will be at `target\release\etwviewer.exe`.

## Usage

```
etwviewer [OPTIONS] <FILE>

Arguments:
  <FILE>  Path to the .etl trace file

Options:
      --local  Use local time for timestamps (default: UTC)
  -h, --help   Print help
```

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `PgDn` / `Ctrl+D` | Page down |
| `PgUp` / `Ctrl+U` | Page up |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| `/` | Enter search mode |
| `n` / `N` | Next / previous search match |
| `f` | Enter filter mode |
| `F` | Clear filter |
| `Enter` | Open detail view |
| `v` | Start visual selection |
| `c` | Copy selected rows |
| `s` | Toggle source column |
| `t` | Cycle timestamp mode |
| `w` | Toggle line wrap |
| `←` / `→` | Horizontal scroll |
| `?` | Show help |
| `q` / `Esc` | Quit / back |

## Dependencies

- [ratatui](https://github.com/ratatui-org/ratatui) — TUI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) — terminal backend
- [clap](https://github.com/clap-rs/clap) — CLI argument parsing
- [regex](https://github.com/rust-lang/regex) — search patterns
- [chrono](https://github.com/chronotope/chrono) — timestamp formatting
- [windows](https://github.com/microsoft/windows-rs) — ETW APIs
