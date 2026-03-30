use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, InputMode};

pub fn render_input_bar(f: &mut Frame, app: &App, area: Rect) {
    let (title, content, style) = match &app.input_mode {
        InputMode::Normal => {
            let time_mode = match app.timestamp_mode {
                crate::app::TimestampMode::Utc => "UTC",
                crate::app::TimestampMode::Local => "Local",
            };
            let event_summary = if let Some(ref filter_desc) = app.active_filter_desc {
                format!("{}/{} events | filter {}", app.visible_count(), app.store.len(), filter_desc)
            } else {
                format!("{} events", app.store.len())
            };
            let parse_summary = if app.should_show_parse_elapsed() {
                if let Some(elapsed) = app.parse_elapsed {
                    format!(" | parsed in {}", format_duration(elapsed))
                } else {
                    String::new()
                }
            } else if app.parse_done {
                String::new()
            } else {
                " | loading".to_string()
            };
            let provider = app.selected_provider_name();
            let provider_str = if provider.is_empty() {
                String::new()
            } else {
                format!(" | {}", provider)
            };
            let status = format!(
                " {} [{}]{}{} | q quit  / search  f filter  F clear  v select  c copy  s src  t time  w wrap  ? help  PgUp/^U PgDn/^D page  ←→ scroll  Enter detail ",
                event_summary, time_mode, provider_str, parse_summary
            );
            ("Status", status, Style::default().fg(Color::Cyan))
        }
        InputMode::Search => {
            let content = format!("/{}", app.input_buffer);
            ("Search", content, Style::default().fg(Color::Yellow))
        }
        InputMode::Filter => {
            let content = format!("filter> {}", app.input_buffer);
            ("Filter", content, Style::default().fg(Color::Green))
        }
        InputMode::Detail => {
            ("Detail", " ↑↓ scroll  PgUp/^U PgDn/^D page  Esc close ".to_string(), Style::default().fg(Color::Magenta))
        }
        InputMode::Visual => {
            let selected = app.table_state.selected().unwrap_or(0);
            let anchor = app.visual_anchor;
            let lo = anchor.min(selected);
            let hi = anchor.max(selected);
            let count = hi - lo + 1;
            let content = format!(" -- VISUAL -- {} lines selected | ↑↓ move  PgUp/^U PgDn/^D page  y yank  Esc cancel ", count);
            ("Visual", content, Style::default().fg(Color::LightMagenta))
        }
        InputMode::Help => {
            ("Help", " Press any key to close ".to_string(), Style::default().fg(Color::Blue))
        }
    };

    let paragraph = Paragraph::new(content)
        .style(style)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(paragraph, area);
}

fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    if secs == 0 {
        format!("{}ms", millis)
    } else {
        format!("{}.{:03}s", secs, millis)
    }
}
