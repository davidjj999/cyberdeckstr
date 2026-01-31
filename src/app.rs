use nostr_sdk::prelude::*;
use serde::Deserialize;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

// Memory limits to prevent unbounded growth
pub const MAX_MESSAGES: usize = 2000;
pub const MAX_SEEN_IDS: usize = 5000;

#[derive(Clone, PartialEq)]
pub enum AppState {
    Login,
    Connecting,
    Feed,
}

pub struct App {
    pub state: AppState,
    pub input: String, // For npub login
    pub messages: VecDeque<String>, // Bounded ring buffer for messages
    pub status: String,
    pub scroll: usize,
    pub client: Option<Client>,
    pub btc_history: Vec<(f64, f64)>,
    // Blockchain Viz State
    pub bitcoin_node_configured: bool,
    pub node_status: String,
    pub blocks: Vec<BlockDisplayInfo>,
    pub mempool: MempoolDisplayInfo,
    pub fees: FeeDisplayInfo,
    pub seen_ids: HashSet<String>,
    pub seen_ids_order: VecDeque<String>, // Track insertion order for LRU eviction
    // Dirty flag for efficient rendering
    pub dirty: bool,
    // Last data received timestamp for health checks
    pub last_nostr_data: Option<std::time::Instant>,
}

#[derive(Clone)]
pub struct BlockDisplayInfo {
    pub height: u64,
    pub size: usize,
    pub timestamp: u64,
    pub tx_count: usize,
}

#[derive(Clone)]
pub struct MempoolDisplayInfo {
    pub size: usize,      // tx count
    pub usage: usize,     // memory usage
    pub max_mempool: usize, 
}

impl Default for MempoolDisplayInfo {
    fn default() -> Self {
        MempoolDisplayInfo { size: 0, usage: 0, max_mempool: 300_000_000 }
    }
}

#[derive(Clone)]
pub struct FeeDisplayInfo {
    pub low: f64,    // sat/vb
    pub medium: f64,
    pub high: f64,
}

impl Default for FeeDisplayInfo {
    fn default() -> Self {
        FeeDisplayInfo { low: 0.0, medium: 0.0, high: 0.0 }
    }
}

#[derive(Deserialize)]
pub struct MarketChart {
    pub prices: Vec<(f64, f64)>,
}

impl App {
    pub fn new() -> App {
        App {
            state: AppState::Login,
            input: String::new(),
            messages: VecDeque::with_capacity(MAX_MESSAGES),
            status: "Welcome to CYBERDECKSTR. Enter NPUB.".to_string(),
            scroll: 0,
            client: None,
            btc_history: Vec::new(),
            bitcoin_node_configured: false,
            node_status: "Node Disconnected".to_string(),
            blocks: Vec::new(),
            mempool: MempoolDisplayInfo::default(),
            fees: FeeDisplayInfo::default(),
            seen_ids: HashSet::with_capacity(MAX_SEEN_IDS),
            seen_ids_order: VecDeque::with_capacity(MAX_SEEN_IDS),
            dirty: true,
            last_nostr_data: None,
        }
    }

    /// Add a message with bounded buffer (ring buffer behavior)
    pub fn add_message(&mut self, id: String, msg: String) -> bool {
        if self.seen_ids.contains(&id) {
            return false;
        }

        // Add to seen_ids with LRU eviction
        if self.seen_ids.len() >= MAX_SEEN_IDS {
            if let Some(old_id) = self.seen_ids_order.pop_front() {
                self.seen_ids.remove(&old_id);
            }
        }
        self.seen_ids.insert(id.clone());
        self.seen_ids_order.push_back(id);

        // Add message with ring buffer eviction
        if self.messages.len() >= MAX_MESSAGES {
            self.messages.pop_front();
            // Adjust scroll if needed
            if self.scroll > 0 {
                self.scroll = self.scroll.saturating_sub(1);
            }
        }
        self.messages.push_back(msg);

        // Auto-scroll to bottom
        if !self.messages.is_empty() {
            self.scroll = self.messages.len() - 1;
        }

        self.dirty = true;
        self.last_nostr_data = Some(std::time::Instant::now());
        true
    }

    /// Mark UI as needing redraw
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Check and clear dirty flag
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }
}

pub type SharedApp = Arc<Mutex<App>>;
