use std::collections::BTreeMap;

/// A parsed ETW event with all display-relevant fields.
#[derive(Clone, Debug)]
pub struct ParsedEvent {
    /// Windows FILETIME: 100-nanosecond intervals since 1601-01-01 UTC.
    pub timestamp: i64,
    pub cpu: u8,
    pub pid: u32,
    pub tid: u32,
    pub provider_name: String,
    pub event_id: u16,
    pub level: u8,
    pub opcode: u8,
    pub message: String,
    /// Byte ranges of substituted field values within `message`: (start, end, color_index).
    pub field_ranges: Vec<(usize, usize, u8)>,
}

/// Append-only event store with a timestamp index for range queries.
pub struct EventStore {
    events: Vec<ParsedEvent>,
    /// Maps timestamp -> list of event indices for efficient time-range lookups.
    timestamp_index: BTreeMap<i64, Vec<usize>>,
}

impl EventStore {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            timestamp_index: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, event: ParsedEvent) {
        let idx = self.events.len();
        let ts = event.timestamp;
        self.events.push(event);
        self.timestamp_index.entry(ts).or_default().push(idx);
    }

    pub fn get(&self, idx: usize) -> Option<&ParsedEvent> {
        self.events.get(idx)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}
