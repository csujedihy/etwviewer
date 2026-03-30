use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;

pub fn render_help(f: &mut Frame, _app: &App) {
    let area = f.area();

    // Clear the screen and draw a centered help box
    let help_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(30), // Height of help box
            Constraint::Fill(1),
        ])
        .split(area)[1];

    let help_area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(80), // Width of help box
            Constraint::Fill(1),
        ])
        .split(help_area)[1];

    let block = Block::default()
        .title("Keybindings")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(Clear, help_area);
    f.render_widget(block, help_area);

    let inner_area = help_area.inner(Margin { vertical: 1, horizontal: 2 });

    let help_text = vec![
        Line::from(vec![
            Span::styled("Navigation:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  j / ↓          Scroll down"),
        Line::from("  k / ↑          Scroll up"),
        Line::from("  PageDown / ^D  Scroll down 30 lines"),
        Line::from("  PageUp / ^U    Scroll up 30 lines"),
        Line::from("  g / Home       Scroll to top"),
        Line::from("  G / End        Scroll to bottom"),
        Line::from("  h / ←          Scroll left"),
        Line::from("  l / →          Scroll right"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Search & Filter:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  /               Enter search mode"),
        Line::from("  n               Next search match"),
        Line::from("  N               Previous search match"),
        Line::from("  f               Enter filter mode"),
        Line::from("  F               Clear filter"),
        Line::from("  Esc             Clear search highlights"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Display:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  t               Toggle timestamp format (UTC/Local)"),
        Line::from("  s               Toggle source column"),
        Line::from("  w               Toggle line wrapping"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Selection & Copy:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  v               Enter visual selection mode"),
        Line::from("  c               Copy selected line to clipboard"),
        Line::from("  C               Copy all visible lines to clipboard"),
        Line::from("  y / c / Enter   Copy visual selection (in visual mode)"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Views:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  Enter           View event details"),
        Line::from("  ?               Show this help"),
        Line::from(""),
        Line::from(vec![
            Span::styled("General:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  q / Ctrl+c      Quit"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press any key to close", Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)),
        ]),
    ];

    let paragraph = Paragraph::new(help_text)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left);

    f.render_widget(paragraph, inner_area);
}