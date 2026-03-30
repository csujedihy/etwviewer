mod app;
mod etw;
mod store;
mod ui;

use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;

use app::{App, InputMode, TimestampMode};

#[derive(Parser)]
#[command(name = "etwviewer", about = "TUI viewer for ETW trace (.etl) files")]
struct Cli {
    /// Path to the ETL file to open
    file: String,

    /// Display timestamps in local time instead of UTC
    #[arg(long, default_value = "false")]
    local: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let parse_started_at = Instant::now();

    let mut app = App::new();
    if cli.local {
        app.timestamp_mode = TimestampMode::Local;
    }

    // Set up channels for background ETL parsing
    let (event_tx, event_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();

    // Spawn parser thread
    let _parse_handle = etw::parser::parse_etl(cli.file, event_tx, done_tx);

    // Set up terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app, event_rx, done_rx, parse_started_at);

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    event_rx: mpsc::Receiver<store::ParsedEvent>,
    done_rx: mpsc::Receiver<()>,
    parse_started_at: Instant,
) -> Result<()> {
    loop {
        // Drain incoming events from parser (non-blocking, batched)
        let mut count = 0;
        while let Ok(evt) = event_rx.try_recv() {
            app.ingest(evt);
            count += 1;
            if count >= 10_000 {
                break; // Yield to UI after batch
            }
        }

        // Check if parsing is done
        if !app.parse_done {
            if done_rx.try_recv().is_ok() {
                // Drain any remaining events
                while let Ok(evt) = event_rx.try_recv() {
                    app.ingest(evt);
                }
                app.parse_done = true;
                app.parse_elapsed = Some(parse_started_at.elapsed());
                app.parse_elapsed_visible_until = Some(Instant::now() + Duration::from_secs(2));
            }
        }

        // Draw UI
        terminal.draw(|f| {
            if app.input_mode == InputMode::Help {
                ui::help::render_help(f, app);
            } else {
                let area = f.area();

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Fill(1), Constraint::Length(3)])
                    .split(area);

                ui::table::render_event_table(f, app, chunks[0]);
                ui::input::render_input_bar(f, app, chunks[1]);
            }
        })?;

        if app.should_quit {
            return Ok(());
        }

        // Handle input (poll with timeout to allow UI updates during parsing)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events, ignore release/repeat
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }
                match &app.input_mode {
                    InputMode::Normal => handle_normal_input(app, key),
                    InputMode::Search => handle_search_input(app, key),
                    InputMode::Filter => handle_filter_input(app, key),
                    InputMode::Detail => handle_detail_input(app, key),
                    InputMode::Visual => handle_visual_input(app, key),
                    InputMode::Help => handle_help_input(app, key),
                }
            }
        }
    }
}

fn handle_normal_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }

        // Navigation
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => app.scroll_down(30),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.scroll_up(30),
        KeyCode::PageDown => app.scroll_down(30),
        KeyCode::PageUp => app.scroll_up(30),
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),
        KeyCode::Left | KeyCode::Char('h') => {
            app.horizontal_scroll = app.horizontal_scroll.saturating_sub(8);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.horizontal_scroll = app.horizontal_scroll.saturating_add(8);
        }

        // Search
        KeyCode::Char('/') => {
            app.input_mode = InputMode::Search;
            app.input_buffer.clear();
        }
        KeyCode::Char('n') => app.next_match(),
        KeyCode::Char('N') => app.prev_match(),

        // Filter
        KeyCode::Char('f') => {
            app.input_mode = InputMode::Filter;
            app.input_buffer.clear();
        }
        KeyCode::Char('F') => {
            // Toggle filter off
            if app.filtered_indices.is_some() {
                app.clear_filter();
            }
        }
        KeyCode::Esc => {
            app.search_regex = None;
            app.search_matches.clear();
            app.search_match_pos = None;
        }

        // Timestamp toggle
        KeyCode::Char('t') => app.toggle_timestamp(),

        // Source column toggle
        KeyCode::Char('s') => {
            app.show_source = !app.show_source;
        }

        // Wrap toggle
        KeyCode::Char('w') => {
            app.wrap_lines = !app.wrap_lines;
        }

        // Visual select mode
        KeyCode::Char('v') => {
            app.visual_anchor = app.table_state.selected().unwrap_or(0);
            app.input_mode = InputMode::Visual;
        }

        // Copy to clipboard
        KeyCode::Char('c') => copy_selected_to_clipboard(app),
        KeyCode::Char('C') => copy_all_visible_to_clipboard(app),

        // Detail view
        KeyCode::Enter => {
            app.detail_scroll = 0;
            app.input_mode = InputMode::Detail;
        }

        // Help
        KeyCode::Char('?') => {
            app.input_mode = InputMode::Help;
        }

        _ => {}
    }
}

fn format_event_line(evt: &crate::store::ParsedEvent, ts_mode: &crate::app::TimestampMode) -> String {
    let ts = match ts_mode {
        crate::app::TimestampMode::Utc => {
            let secs = (evt.timestamp / 10_000_000) - 11_644_473_600;
            let nanos = ((evt.timestamp % 10_000_000) * 100) as u32;
            chrono::DateTime::from_timestamp(secs, nanos)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
                .unwrap_or_default()
        }
        crate::app::TimestampMode::Local => {
            let secs = (evt.timestamp / 10_000_000) - 11_644_473_600;
            let nanos = ((evt.timestamp % 10_000_000) * 100) as u32;
            chrono::DateTime::from_timestamp(secs, nanos)
                .map(|dt| {
                    use chrono::TimeZone;
                    chrono::Local.from_utc_datetime(&dt.naive_utc())
                        .format("%Y-%m-%d %H:%M:%S%.6f")
                        .to_string()
                })
                .unwrap_or_default()
        }
    };
    format!("{}  {}", ts, evt.message)
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("clip.exe")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

fn copy_selected_to_clipboard(app: &mut App) {
    let indices = app.visible_indices().to_vec();
    let selected = app.table_state.selected().unwrap_or(0);
    if selected < indices.len() {
        if let Some(evt) = app.store.get(indices[selected]) {
            let line = format_event_line(evt, &app.timestamp_mode);
            copy_to_clipboard(&line);
        }
    }
}

fn copy_all_visible_to_clipboard(app: &mut App) {
    let indices = app.visible_indices().to_vec();
    let mut lines = String::new();
    for idx in &indices {
        if let Some(evt) = app.store.get(*idx) {
            lines.push_str(&format_event_line(evt, &app.timestamp_mode));
            lines.push('\n');
        }
    }
    copy_to_clipboard(&lines);
}

fn copy_visual_selection_to_clipboard(app: &mut App) {
    let indices = app.visible_indices().to_vec();
    let selected = app.table_state.selected().unwrap_or(0);
    let anchor = app.visual_anchor;
    let lo = anchor.min(selected);
    let hi = anchor.max(selected);
    let mut lines = String::new();
    for i in lo..=hi {
        if i < indices.len() {
            if let Some(evt) = app.store.get(indices[i]) {
                lines.push_str(&format_event_line(evt, &app.timestamp_mode));
                lines.push('\n');
            }
        }
    }
    copy_to_clipboard(&lines);
    app.input_mode = InputMode::Normal;
}

fn handle_visual_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        // Navigation — extend selection
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => app.scroll_down(30),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.scroll_up(30),
        KeyCode::PageDown => app.scroll_down(30),
        KeyCode::PageUp => app.scroll_up(30),
        KeyCode::Home | KeyCode::Char('g') => app.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.scroll_to_bottom(),

        // Yank (copy) selection
        KeyCode::Char('y') | KeyCode::Char('c') | KeyCode::Enter => {
            copy_visual_selection_to_clipboard(app);
        }

        // Cancel
        KeyCode::Esc | KeyCode::Char('v') | KeyCode::Char('q') => {
            app.input_mode = InputMode::Normal;
        }

        _ => {}
    }
}

fn handle_search_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.apply_search();
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buffer.clear();
        }
        KeyCode::Backspace => {
            app.input_buffer.pop();
        }
        KeyCode::Char(c) => {
            app.input_buffer.push(c);
        }
        _ => {}
    }
}

fn handle_filter_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.apply_filter();
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buffer.clear();
        }
        KeyCode::Backspace => {
            app.input_buffer.pop();
        }
        KeyCode::Char(c) => {
            app.input_buffer.push(c);
        }
        _ => {}
    }
}

fn handle_detail_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.detail_scroll = app.detail_scroll.saturating_add(10);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.detail_scroll = app.detail_scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            app.detail_scroll = app.detail_scroll.saturating_add(10);
        }
        KeyCode::PageUp => {
            app.detail_scroll = app.detail_scroll.saturating_sub(10);
        }
        _ => {}
    }
}

fn handle_help_input(app: &mut App, _key: event::KeyEvent) {
    app.input_mode = InputMode::Normal;
}
