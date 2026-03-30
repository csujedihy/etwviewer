use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, Wrap, Paragraph, Clear};
use ratatui::Frame;
use regex::Regex;

use crate::app::{App, InputMode, TimestampMode};

/// Horizontally shift a highlighted Line by `offset` characters.
fn hscroll_line(line: Line<'_>, offset: usize) -> Line<'_> {
    if offset == 0 {
        return line;
    }
    let mut remaining = offset;
    let mut new_spans: Vec<Span<'_>> = Vec::new();
    for span in line.spans {
        if remaining == 0 {
            new_spans.push(span);
            continue;
        }
        let char_count = span.content.chars().count();
        if remaining >= char_count {
            remaining -= char_count;
            continue;
        }
        let byte_start = span.content.char_indices()
            .nth(remaining)
            .map(|(i, _)| i)
            .unwrap_or(span.content.len());
        let sliced: &str = &span.content[byte_start..];
        new_spans.push(Span::styled(sliced.to_string(), span.style));
        remaining = 0;
    }
    Line::from(new_spans)
}

/// Wrap a styled Line into multiple Lines at `width` character boundaries.
/// Splits spans across line boundaries while preserving styles.
fn wrap_line(line: Line<'static>, width: usize) -> Text<'static> {
    if width == 0 {
        return Text::from(line);
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;

    for span in line.spans {
        let style = span.style;
        let content: &str = &span.content;
        let mut remaining = content;

        while !remaining.is_empty() {
            let chars_left = width - col;
            let char_count = remaining.chars().count();

            if char_count <= chars_left {
                // Whole span fits on current line
                current_spans.push(Span::styled(remaining.to_string(), style));
                col += char_count;
                break;
            } else {
                // Split this span at the boundary
                let byte_split = remaining.char_indices()
                    .nth(chars_left)
                    .map(|(i, _)| i)
                    .unwrap_or(remaining.len());
                let (before, after) = remaining.split_at(byte_split);
                if !before.is_empty() {
                    current_spans.push(Span::styled(before.to_string(), style));
                }
                lines.push(Line::from(std::mem::take(&mut current_spans)));
                col = 0;
                remaining = after;
            }
        }
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    Text::from(lines)
}

pub fn render_event_table(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    const TIMESTAMP_COL_WIDTH: u16 = 26;

    let visible_indices = app.visible_indices().to_vec();
    let total = visible_indices.len();
    let (table_area, indicator_area) = if area.width > 24 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    if app.table_state.selected().is_none() && total > 0 {
        app.table_state.select(Some(0));
    }

    let view_height = table_area.height as usize;
    let selected = app.table_state.selected().unwrap_or(0);
    let offset = app.table_state.offset();

    let effective_offset = if selected < offset {
        selected
    } else if selected >= offset + view_height {
        selected.saturating_sub(view_height - 1)
    } else {
        offset
    };

    let render_start = effective_offset;
    let render_end = (effective_offset + view_height + 4).min(total);

    if render_start > 0 {
        *app.table_state.offset_mut() = render_start;
    } else {
        *app.table_state.offset_mut() = 0;
    }

    let adjusted_selected = selected.saturating_sub(render_start);
    let mut partial_state = ratatui::widgets::TableState::default();
    partial_state.select(Some(adjusted_selected));
    let active_search_row = app
        .search_match_pos
        .and_then(|pos| app.search_matches.get(pos))
        .copied();

    // Visual selection range (row indices in visible list)
    let visual_range = app.visual_range();

    // Compute message column width for wrapping
    let msg_col_width = if app.show_source {
        table_area.width.saturating_sub(TIMESTAMP_COL_WIDTH + 18 + 3) as usize // time + source + column gaps
    } else {
        table_area.width.saturating_sub(TIMESTAMP_COL_WIDTH + 2) as usize // time + column gap
    };

    if app.show_source {
        let header = Row::new(vec![
            Cell::from("Time"),
            Cell::from("Source"),
            Cell::from("Message"),
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

        let mut rows: Vec<Row> = Vec::with_capacity(render_end - render_start);
        for (p, &idx) in visible_indices[render_start..render_end].iter().enumerate() {
            let vis_row = render_start + p;
            let evt = app.store.get(idx).unwrap();
            let ts = format_timestamp(evt.timestamp, &app.timestamp_mode);
            let source = format!(
                "[{}]{:X}.{:X}",
                evt.cpu, evt.pid, evt.tid
            );
            let msg = styled_message(
                &evt.message,
                &evt.field_ranges,
                app.search_regex.as_ref(),
                app.horizontal_scroll,
            );
            let mut row = if app.wrap_lines && msg_col_width > 0 {
                let wrapped = wrap_line(msg, msg_col_width);
                let h = wrapped.lines.len() as u16;
                Row::new(vec![
                    Cell::from(ts),
                    Cell::from(source),
                    Cell::from(wrapped),
                ]).height(h)
            } else {
                Row::new(vec![
                    Cell::from(ts),
                    Cell::from(source),
                    Cell::from(msg),
                ])
            };
            if let Some((lo, hi)) = visual_range {
                if vis_row >= lo && vis_row <= hi {
                    row = row.style(Style::default().bg(Color::Indexed(236)));
                }
            }
            rows.push(row);
        }

        let widths = [
            Constraint::Length(TIMESTAMP_COL_WIDTH),
            Constraint::Length(18),
            Constraint::Fill(1),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(if active_search_row == Some(selected) {
                Style::default().bg(Color::Indexed(24))
            } else {
                Style::default().bg(Color::DarkGray)
            });
        f.render_stateful_widget(table, table_area, &mut partial_state);
    } else {
        let header = Row::new(vec![
            Cell::from("Time"),
            Cell::from("Message"),
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

        let mut rows: Vec<Row> = Vec::with_capacity(render_end - render_start);
        for (p, &idx) in visible_indices[render_start..render_end].iter().enumerate() {
            let vis_row = render_start + p;
            let evt = app.store.get(idx).unwrap();
            let ts = format_timestamp(evt.timestamp, &app.timestamp_mode);
            let msg = styled_message(
                &evt.message,
                &evt.field_ranges,
                app.search_regex.as_ref(),
                app.horizontal_scroll,
            );
            let mut row = if app.wrap_lines && msg_col_width > 0 {
                let wrapped = wrap_line(msg, msg_col_width);
                let h = wrapped.lines.len() as u16;
                Row::new(vec![
                    Cell::from(ts),
                    Cell::from(wrapped),
                ]).height(h)
            } else {
                Row::new(vec![
                    Cell::from(ts),
                    Cell::from(msg),
                ])
            };
            if let Some((lo, hi)) = visual_range {
                if vis_row >= lo && vis_row <= hi {
                    row = row.style(Style::default().bg(Color::Indexed(236)));
                }
            }
            rows.push(row);
        }

        let widths = [
            Constraint::Length(TIMESTAMP_COL_WIDTH),
            Constraint::Fill(1),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(if active_search_row == Some(selected) {
                Style::default().bg(Color::Indexed(24))
            } else {
                Style::default().bg(Color::DarkGray)
            });
        f.render_stateful_widget(table, table_area, &mut partial_state);
    }

    if let Some(indicator_area) = indicator_area {
        render_position_indicator(f, indicator_area, total, selected);
    }

    if app.input_mode == InputMode::Detail {
        render_detail_popup(f, app, &visible_indices);
    }
}

fn render_position_indicator(f: &mut Frame, area: Rect, total: usize, selected: usize) {
    if area.width == 0 || area.height <= 1 || total == 0 {
        return;
    }

    let track_height = area.height.saturating_sub(1) as usize;
    if track_height == 0 {
        return;
    }

    let marker_row = if total <= 1 {
        0
    } else {
        selected.saturating_mul(track_height.saturating_sub(1)) / (total - 1)
    };

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(area.height as usize);
    lines.push(Line::from(Span::raw(" ")));

    for row in 0..track_height {
        let span = if row == marker_row {
            Span::styled("-", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("|", Style::default().fg(Color::DarkGray))
        };
        lines.push(Line::from(span));
    }

    f.render_widget(Paragraph::new(lines), area);
}

const FIELD_COLORS: [Color; 8] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightMagenta,
];

const SEARCH_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Yellow);

/// Build a styled Line for a message string.
/// Field values are coloured according to the supplied byte-ranges,
/// search matches are highlighted, and horizontal scroll is applied.
fn styled_message(
    text: &str,
    field_ranges: &[(usize, usize, u8)],
    search_re: Option<&Regex>,
    hscroll: usize,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut pos: usize = 0;

    for &(start, end, color_idx) in field_ranges {
        // Clamp to valid byte boundaries
        let s = start.min(text.len());
        let e = end.min(text.len());
        if s < pos || s >= e {
            continue;
        }
        // Template text before this field
        if pos < s {
            spans.push(Span::raw(text[pos..s].to_string()));
        }
        let color = FIELD_COLORS[color_idx as usize % FIELD_COLORS.len()];
        spans.push(Span::styled(text[s..e].to_string(), Style::new().fg(color)));
        pos = e;
    }

    // Trailing template text
    if pos < text.len() {
        spans.push(Span::raw(text[pos..].to_string()));
    }

    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }

    if let Some(re) = search_re {
        spans = apply_search_highlight(spans, re);
    }

    hscroll_line(Line::from(spans), hscroll)
}

/// Overlay search match highlighting onto existing styled spans.
fn apply_search_highlight(spans: Vec<Span<'static>>, re: &Regex) -> Vec<Span<'static>> {
    let mut result: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let base_style = span.style;
        let text: &str = &span.content;
        let mut last_end = 0;

        for m in re.find_iter(text) {
            if m.start() > last_end {
                result.push(Span::styled(text[last_end..m.start()].to_string(), base_style));
            }
            // Merge search highlight with existing style (search bg+fg wins)
            result.push(Span::styled(
                text[m.start()..m.end()].to_string(),
                SEARCH_STYLE.add_modifier(Modifier::BOLD),
            ));
            last_end = m.end();
        }

        if last_end < text.len() {
            result.push(Span::styled(text[last_end..].to_string(), base_style));
        } else if last_end == 0 {
            // No matches in this span — keep as-is
            result.push(span);
        }
    }

    result
}

/// Render a centered popup showing the full details of the selected event.
fn render_detail_popup(f: &mut Frame, app: &App, visible_indices: &[usize]) {
    let selected = app.table_state.selected().unwrap_or(0);
    if selected >= visible_indices.len() {
        return;
    }
    let event_idx = visible_indices[selected];
    let evt = match app.store.get(event_idx) {
        Some(e) => e,
        None => return,
    };

    let screen = f.area();
    // Use most of the screen
    let popup_w = screen.width.saturating_sub(8).max(40);
    let popup_h = screen.height.saturating_sub(6).max(10);
    let popup_x = (screen.width.saturating_sub(popup_w)) / 2;
    let popup_y = (screen.height.saturating_sub(popup_h)) / 2;
    let popup_area = ratatui::layout::Rect::new(popup_x, popup_y, popup_w, popup_h);

    let ts_full = format_timestamp_full(evt.timestamp, &app.timestamp_mode);
    let text = format!(
        "Timestamp: {}\n\
         CPU: {}  PID: {:X} ({})  TID: {:X} ({})\n\
         Provider:  {}\n\
         Event ID:  {}  Level: {} ({})  Opcode: {}\n\
         \n\
         {}",
        ts_full,
        evt.cpu, evt.pid, evt.pid, evt.tid, evt.tid,
        evt.provider_name,
        evt.event_id, level_str(evt.level), evt.level, evt.opcode,
        evt.message,
    );

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Event Detail (Esc to close, ↑↓ scroll) ")
                .style(Style::default().bg(Color::Black)),
        )
        .style(Style::default().fg(Color::White).bg(Color::Black));

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}

fn format_timestamp(filetime: i64, mode: &TimestampMode) -> String {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let unix_100ns = filetime - EPOCH_DIFF;
    let secs = unix_100ns / 10_000_000;
    let nanos = ((unix_100ns % 10_000_000) * 100) as u32;

    match mode {
        TimestampMode::Utc => {
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) {
                dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
            } else {
                format!("0x{:016x}", filetime)
            }
        }
        TimestampMode::Local => {
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) {
                let local: chrono::DateTime<chrono::Local> = dt.into();
                local.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
            } else {
                format!("0x{:016x}", filetime)
            }
        }
    }
}

fn format_timestamp_full(filetime: i64, mode: &TimestampMode) -> String {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let unix_100ns = filetime - EPOCH_DIFF;
    let secs = unix_100ns / 10_000_000;
    let nanos = ((unix_100ns % 10_000_000) * 100) as u32;

    match mode {
        TimestampMode::Utc => {
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) {
                dt.format("%Y-%m-%d %H:%M:%S%.9f UTC").to_string()
            } else {
                format!("0x{:016x}", filetime)
            }
        }
        TimestampMode::Local => {
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) {
                let local: chrono::DateTime<chrono::Local> = dt.into();
                local.format("%Y-%m-%d %H:%M:%S%.9f %Z").to_string()
            } else {
                format!("0x{:016x}", filetime)
            }
        }
    }
}

fn level_str(level: u8) -> &'static str {
    match level {
        1 => "Critical",
        2 => "Error",
        3 => "Warning",
        4 => "Info",
        5 => "Verbose",
        0 => "None",
        _ => "?",
    }
}
