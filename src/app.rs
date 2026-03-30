use std::time::{Duration, Instant};

use ratatui::widgets::TableState;
use regex::Regex;

use crate::store::{EventStore, ParsedEvent};

#[derive(Clone, Debug, PartialEq)]
pub enum TimestampMode {
    Utc,
    Local,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    Filter,
    Detail,
    Visual,
    Help,
}

pub struct App {
    pub store: EventStore,
    pub table_state: TableState,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub timestamp_mode: TimestampMode,
    pub parse_done: bool,
    pub parse_elapsed: Option<Duration>,
    pub parse_elapsed_visible_until: Option<Instant>,
    pub should_quit: bool,

    // Search
    pub search_regex: Option<Regex>,
    pub search_matches: Vec<usize>,   // indices into visible_indices that match
    pub search_match_pos: Option<usize>, // current position in search_matches

    // Filter
    pub filtered_indices: Option<Vec<usize>>,
    pub active_filter_desc: Option<String>,

    // Cached visible indices
    visible_cache: Option<Vec<usize>>,
    last_store_len: usize,

    // Detail view scroll offset
    pub detail_scroll: u16,

    // UI toggles
    pub show_source: bool,
    pub horizontal_scroll: usize,
    pub wrap_lines: bool,

    // Visual selection anchor (row index in visible_indices)
    pub visual_anchor: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            store: EventStore::new(),
            table_state: TableState::default(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            timestamp_mode: TimestampMode::Utc,
            parse_done: false,
            parse_elapsed: None,
            parse_elapsed_visible_until: None,
            should_quit: false,
            search_regex: None,
            search_matches: Vec::new(),
            search_match_pos: None,
            filtered_indices: None,
            active_filter_desc: None,
            visible_cache: None,
            last_store_len: 0,
            detail_scroll: 0,
            show_source: false,
            horizontal_scroll: 0,
            wrap_lines: false,
            visual_anchor: 0,
        }
    }

    /// Invalidate the visible indices cache.
    fn invalidate_cache(&mut self) {
        self.visible_cache = None;
    }

    pub fn ingest(&mut self, event: ParsedEvent) {
        self.store.push(event);
        if self.filtered_indices.is_some() {
            self.visible_cache = None;
        }
    }

    /// Returns the event indices to display, considering filter.
    /// Uses a cache that is invalidated when filter/store changes.
    /// Returns a borrowed slice to avoid cloning.
    pub fn visible_indices(&mut self) -> &[usize] {
        let store_len = self.store.len();

        // Check if cache is still valid
        let cache_valid = if let Some(ref cache) = self.visible_cache {
            if self.filtered_indices.is_none() {
                cache.len() == store_len
            } else {
                self.last_store_len == store_len
            }
        } else {
            false
        };

        if !cache_valid {
            let result = if let Some(ref filtered) = self.filtered_indices {
                filtered.clone()
            } else {
                (0..store_len).collect()
            };
            self.visible_cache = Some(result);
            self.last_store_len = store_len;
        }

        self.visible_cache.as_deref().unwrap()
    }

    pub fn visible_count(&self) -> usize {
        self.filtered_indices
            .as_ref()
            .map(|indices| indices.len())
            .unwrap_or_else(|| self.store.len())
    }

    pub fn should_show_parse_elapsed(&self) -> bool {
        self.parse_done
            && self.parse_elapsed.is_some()
            && self
                .parse_elapsed_visible_until
                .map(|deadline| Instant::now() <= deadline)
                .unwrap_or(false)
    }

    // --- Navigation ---

    /// Returns the visual selection range (lo, hi) in visible row indices,
    /// or None if not in visual mode.
    pub fn visual_range(&self) -> Option<(usize, usize)> {
        if self.input_mode != InputMode::Visual {
            return None;
        }
        let selected = self.table_state.selected().unwrap_or(0);
        let lo = self.visual_anchor.min(selected);
        let hi = self.visual_anchor.max(selected);
        Some((lo, hi))
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let total = self.visible_indices().len();
        if total == 0 {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = (current + amount).min(total - 1);
        self.table_state.select(Some(next));
    }

    pub fn scroll_up(&mut self, amount: usize) {
        let current = self.table_state.selected().unwrap_or(0);
        let next = current.saturating_sub(amount);
        self.table_state.select(Some(next));
    }

    pub fn scroll_to_top(&mut self) {
        if !self.visible_indices().is_empty() {
            self.table_state.select(Some(0));
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        let total = self.visible_indices().len();
        if total > 0 {
            self.table_state.select(Some(total - 1));
        }
    }

    // --- Search ---

    pub fn apply_search(&mut self) {
        let pattern = self.input_buffer.clone();
        if pattern.is_empty() {
            self.search_regex = None;
            self.search_matches.clear();
            self.search_match_pos = None;
            return;
        }

        match Regex::new(&pattern) {
            Ok(re) => {
                let visible: Vec<usize> = self.visible_indices().to_vec();
                let matches: Vec<usize> = visible
                    .iter()
                    .enumerate()
                    .filter(|(_, idx)| {
                        let evt = self.store.get(**idx).unwrap();
                        re.is_match(&evt.message)
                            || re.is_match(&evt.provider_name)
                    })
                    .map(|(pos, _)| pos)
                    .collect();
                self.search_regex = Some(re);
                self.search_matches = matches;
                if !self.search_matches.is_empty() {
                    self.search_match_pos = Some(0);
                    let target = self.search_matches[0];
                    self.table_state.select(Some(target));
                } else {
                    self.search_match_pos = None;
                }
            }
            Err(_) => {
                // Invalid regex - treat as literal
                let escaped = regex::escape(&pattern);
                if let Ok(re) = Regex::new(&escaped) {
                    let visible: Vec<usize> = self.visible_indices().to_vec();
                    let matches: Vec<usize> = visible
                        .iter()
                        .enumerate()
                        .filter(|(_, idx)| {
                            let evt = self.store.get(**idx).unwrap();
                            re.is_match(&evt.message)
                                || re.is_match(&evt.provider_name)
                        })
                        .map(|(pos, _)| pos)
                        .collect();
                    self.search_regex = Some(re);
                    self.search_matches = matches;
                    if !self.search_matches.is_empty() {
                        self.search_match_pos = Some(0);
                        let target = self.search_matches[0];
                        self.table_state.select(Some(target));
                    }
                }
            }
        }
    }

    pub fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let pos = self
            .search_match_pos
            .map(|p| (p + 1) % self.search_matches.len())
            .unwrap_or(0);
        self.search_match_pos = Some(pos);
        let target = self.search_matches[pos];
        self.table_state.select(Some(target));
    }

    pub fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let pos = self
            .search_match_pos
            .map(|p| {
                if p == 0 {
                    self.search_matches.len() - 1
                } else {
                    p - 1
                }
            })
            .unwrap_or(0);
        self.search_match_pos = Some(pos);
        let target = self.search_matches[pos];
        self.table_state.select(Some(target));
    }

    // --- Filter ---

    pub fn apply_filter(&mut self) {
        let input = self.input_buffer.trim().to_string();
        if input.is_empty() {
            self.clear_filter();
            return;
        }

        let mut provider_re: Option<Regex> = None;
        let mut level_filter: Option<u8> = None;
        let mut pid_filter: Option<u32> = None;
        let mut msg_re: Option<Regex> = None;
        let mut generic_re: Option<Regex> = None;

        // Parse filter expressions: provider:X level:X pid:X msg:X
        // If no prefix, treat as generic message filter
        let mut has_prefix = false;
        for part in input.split_whitespace() {
            if let Some(val) = part.strip_prefix("provider:") {
                provider_re = Regex::new(&format!("(?i){}", regex::escape(val))).ok();
                has_prefix = true;
            } else if let Some(val) = part.strip_prefix("level:") {
                level_filter = match val.to_lowercase().as_str() {
                    "critical" | "1" => Some(1),
                    "error" | "2" => Some(2),
                    "warning" | "3" => Some(3),
                    "info" | "4" => Some(4),
                    "verbose" | "5" => Some(5),
                    _ => None,
                };
                has_prefix = true;
            } else if let Some(val) = part.strip_prefix("pid:") {
                pid_filter = val.parse().ok();
                has_prefix = true;
            } else if let Some(val) = part.strip_prefix("msg:") {
                msg_re = Regex::new(&format!("(?i){}", val)).ok();
                has_prefix = true;
            }
        }

        if !has_prefix {
            // Treat entire input as a case-insensitive message/provider regex
            generic_re = Regex::new(&format!("(?i){}", regex::escape(&input))).ok();
        }

        let total = self.store.len();
        let mut indices = Vec::new();

        for i in 0..total {
            let evt = self.store.get(i).unwrap();

            if let Some(ref re) = provider_re {
                if !re.is_match(&evt.provider_name) {
                    continue;
                }
            }
            if let Some(lvl) = level_filter {
                if evt.level != lvl {
                    continue;
                }
            }
            if let Some(pid) = pid_filter {
                if evt.pid != pid {
                    continue;
                }
            }
            if let Some(ref re) = msg_re {
                if !re.is_match(&evt.message) {
                    continue;
                }
            }
            if let Some(ref re) = generic_re {
                if !re.is_match(&evt.message) && !re.is_match(&evt.provider_name) {
                    continue;
                }
            }

            indices.push(i);
        }

        self.filtered_indices = Some(indices);
        self.active_filter_desc = Some(input);
        self.invalidate_cache();
        self.table_state.select(Some(0));
    }

    pub fn clear_filter(&mut self) {
        self.filtered_indices = None;
        self.active_filter_desc = None;
        self.invalidate_cache();
        self.table_state.select(Some(0));
    }

    // --- Timestamp toggle ---

    pub fn toggle_timestamp(&mut self) {
        self.timestamp_mode = match self.timestamp_mode {
            TimestampMode::Utc => TimestampMode::Local,
            TimestampMode::Local => TimestampMode::Utc,
        };
    }

    /// Returns the provider name of the currently selected event.
    pub fn selected_provider_name(&self) -> &str {
        let selected = self.table_state.selected().unwrap_or(0);
        let indices = self.visible_cache.as_deref().unwrap_or(&[]);
        if let Some(&idx) = indices.get(selected) {
            if let Some(evt) = self.store.get(idx) {
                return &evt.provider_name;
            }
        }
        ""
    }
}
