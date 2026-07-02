use nostr_sdk::prelude::*;
use std::collections::{HashSet, VecDeque};

// Memory limits to prevent unbounded growth
pub const MAX_MESSAGES: usize = 2000;
pub const MAX_SEEN_IDS: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeedEntryKind {
    TextNote,
    Repost,
}

/// A single entry in the Nostr feed with all metadata needed for rich rendering.
#[derive(Clone)]
pub struct FeedEntry {
    pub id: EventId,
    pub author: String,
    pub kind: FeedEntryKind,
    pub content: String,
    pub is_reply: bool,
    pub nip05: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum AppState {
    Login,
    Connecting,
    Feed,
}

// ---------------------------------------------------------------------------
// AppMessage — all background tasks communicate through this enum
// ---------------------------------------------------------------------------

/// Messages sent from background tasks to the main loop.
/// The main loop is the sole owner of `App`; no locks required.
pub enum AppMessage {
    /// A new Nostr event to display (text note or repost)
    NostrEvent {
        id: EventId,
        author: String,
        kind: FeedEntryKind,
        content: String,
        is_reply: bool,
        nip05: Option<String>,
    },
    /// Status update from the Nostr connection task
    NostrStatus(String),
    /// Nostr connection established — transition to Feed state
    NostrConnected,
    /// Nostr connection error — return to Login
    NostrLoginError(String),
    /// Updated BTC price history from CoinGecko
    BtcPriceUpdate(Vec<(f64, f64)>),
    /// Updated blockchain data from Bitcoin Core
    BlockchainUpdate {
        blocks: Vec<BlockDisplayInfo>,
        mempool: MempoolDisplayInfo,
        fees: FeeDisplayInfo,
    },
    /// Status update from the blockchain poller
    BlockchainStatus(String),
    /// Updated system monitor stats
    SystemStatsUpdate {
        cpu: u8,
        gpu: u8,
        ram: u8,
        vram: u8,
        network: String,
    },
}

// ---------------------------------------------------------------------------
// Sub-states
// ---------------------------------------------------------------------------

/// Cached chart axis bounds, recomputed only when price data changes.
#[derive(Clone)]
pub struct ChartBounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl Default for ChartBounds {
    fn default() -> Self {
        ChartBounds {
            min_x: 0.0,
            max_x: 100.0,
            min_y: 0.0,
            max_y: 100.0,
        }
    }
}

/// Nostr feed state: entries, dedup set, and text-wrap cache.
pub struct FeedState {
    pub entries: VecDeque<FeedEntry>,
    pub seen_ids: HashSet<EventId>,
    pub seen_ids_order: VecDeque<EventId>,
    pub last_nostr_data: Option<std::time::Instant>,
    /// Pre-wrapped content lines, kept in sync with `entries`.
    pub wrapped_content: VecDeque<Vec<String>>,
    /// Terminal width the cache was built at. 0 = not yet known.
    pub cached_wrap_width: usize,
}

impl FeedState {
    pub fn new() -> Self {
        FeedState {
            entries: VecDeque::with_capacity(MAX_MESSAGES),
            seen_ids: HashSet::with_capacity(MAX_SEEN_IDS),
            seen_ids_order: VecDeque::with_capacity(MAX_SEEN_IDS),
            last_nostr_data: None,
            wrapped_content: VecDeque::with_capacity(MAX_MESSAGES),
            cached_wrap_width: 0,
        }
    }

    /// Add an entry. Returns `true` if it was new (not a duplicate).
    pub fn add_entry(&mut self, entry: FeedEntry) -> bool {
        if self.seen_ids.contains(&entry.id) {
            return false;
        }

        // LRU eviction on the dedup set
        if self.seen_ids.len() >= MAX_SEEN_IDS {
            if let Some(old_id) = self.seen_ids_order.pop_front() {
                self.seen_ids.remove(&old_id);
            }
        }
        self.seen_ids.insert(entry.id);
        self.seen_ids_order.push_back(entry.id);

        // Pre-wrap the content at the current cached width
        let wrapped = if self.cached_wrap_width > 0 {
            wrap_message(&entry.content, self.cached_wrap_width)
        } else {
            // Width not yet known — store raw as single-line fallback
            vec![entry.content.clone()]
        };

        // Ring buffer eviction
        if self.entries.len() >= MAX_MESSAGES {
            self.entries.pop_front();
            self.wrapped_content.pop_front();
        }
        self.entries.push_back(entry);
        self.wrapped_content.push_back(wrapped);

        self.last_nostr_data = Some(std::time::Instant::now());
        true
    }

    /// Re-wrap all entry content at a new terminal width.
    pub fn rewrap(&mut self, width: usize) {
        if width == self.cached_wrap_width || width == 0 {
            return;
        }
        self.cached_wrap_width = width;
        self.wrapped_content.clear();
        for entry in &self.entries {
            self.wrapped_content.push_back(wrap_message(&entry.content, width));
        }
    }
}

/// Wrap a single message string into display lines.
fn wrap_message(msg: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in msg.lines() {
        if line.is_empty() {
            lines.push(String::new());
            continue;
        }
        for w in textwrap::wrap(line, max_width) {
            lines.push(w.to_string());
        }
    }
    lines
}

/// BTC market price state with cached chart bounds.
pub struct MarketState {
    pub btc_history: Vec<(f64, f64)>,
    pub chart_bounds: ChartBounds,
}

impl MarketState {
    pub fn new() -> Self {
        MarketState {
            btc_history: Vec::new(),
            chart_bounds: ChartBounds::default(),
        }
    }

    /// Replace price data and recompute axis bounds.
    pub fn update_prices(&mut self, prices: Vec<(f64, f64)>) {
        if prices.is_empty() {
            self.chart_bounds = ChartBounds::default();
        } else {
            let min_x = prices.first().map(|(x, _)| *x).unwrap_or(0.0);
            let max_x = prices.last().map(|(x, _)| *x).unwrap_or(100.0);
            let min_y = prices.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
            let max_y = prices.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max);
            self.chart_bounds = ChartBounds { min_x, max_x, min_y, max_y };
        }
        self.btc_history = prices;
    }
}

/// Bitcoin node state (present only when a node is configured).
pub struct NodeState {
    pub status: String,
    pub blocks: Vec<BlockDisplayInfo>,
    pub mempool: MempoolDisplayInfo,
    pub fees: FeeDisplayInfo,
}

impl NodeState {
    pub fn new() -> Self {
        NodeState {
            status: "CONNECTING TO NODE...".to_string(),
            blocks: Vec::new(),
            mempool: MempoolDisplayInfo::default(),
            fees: FeeDisplayInfo::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Display structs for blockchain data
// ---------------------------------------------------------------------------

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
    pub usage: usize,     // memory usage in bytes
    pub max_mempool: usize,
}

impl Default for MempoolDisplayInfo {
    fn default() -> Self {
        MempoolDisplayInfo { size: 0, usage: 0, max_mempool: 300_000_000 }
    }
}

#[derive(Clone)]
pub struct FeeDisplayInfo {
    pub low: f64,    // sat/vB
    pub medium: f64,
    pub high: f64,
}

impl Default for FeeDisplayInfo {
    fn default() -> Self {
        FeeDisplayInfo { low: 0.0, medium: 0.0, high: 0.0 }
    }
}

#[derive(Clone, Debug)]
pub struct SystemStats {
    pub cpu: u8,
    pub gpu: u8,
    pub ram: u8,
    pub vram: u8,
    pub network: String,
}

// ---------------------------------------------------------------------------
// App — top-level state, owned exclusively by the main loop
// ---------------------------------------------------------------------------

pub struct App {
    pub state: AppState,
    pub input: String,       // npub input on Login screen
    pub scroll: usize,
    pub status: String,
    pub dirty: bool,

    pub feed: FeedState,
    pub market: MarketState,
    pub node: Option<NodeState>,
    pub system_stats: Option<SystemStats>,
}

impl App {
    pub fn new() -> App {
        App {
            state: AppState::Login,
            input: String::new(),
            scroll: 0,
            status: "Welcome to CYBERDECKSTR. Enter NPUB.".to_string(),
            dirty: true,
            feed: FeedState::new(),
            market: MarketState::new(),
            node: None,
            system_stats: None,
        }
    }

    /// Process a message from a background task.
    pub fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::NostrEvent { id, author, kind, content, is_reply, nip05 } => {
                // Track whether an entry was evicted for scroll adjustment
                let will_evict = self.feed.entries.len() >= MAX_MESSAGES;
                let was_at_bottom = self.feed.entries.is_empty()
                    || self.scroll >= self.feed.entries.len().saturating_sub(1);

                let entry = FeedEntry { id, author, kind, content, is_reply, nip05 };
                if self.feed.add_entry(entry) {
                    if will_evict && self.scroll > 0 {
                        self.scroll = self.scroll.saturating_sub(1);
                    }
                    // Auto-scroll to bottom if user was already there
                    if was_at_bottom && !self.feed.entries.is_empty() {
                        self.scroll = self.feed.entries.len() - 1;
                    }
                    self.dirty = true;
                }
            }
            AppMessage::NostrStatus(status) => {
                self.status = status;
                self.dirty = true;
            }
            AppMessage::NostrConnected => {
                self.state = AppState::Feed;
                self.status = "CONNECTED. SIGNAL ACQUIRED.".to_string();
                self.dirty = true;
            }
            AppMessage::NostrLoginError(msg) => {
                self.status = msg;
                self.state = AppState::Login;
                self.input.clear();
                self.dirty = true;
            }
            AppMessage::BtcPriceUpdate(prices) => {
                self.market.update_prices(prices);
                self.dirty = true;
            }
            AppMessage::BlockchainUpdate { blocks, mempool, fees } => {
                if let Some(ref mut node) = self.node {
                    node.blocks = blocks;
                    node.mempool = mempool;
                    node.fees = fees;
                    node.status = "NODE CONNECTED".to_string();
                }
                self.dirty = true;
            }
            AppMessage::BlockchainStatus(status) => {
                if let Some(ref mut node) = self.node {
                    node.status = status;
                }
                self.dirty = true;
            }
            AppMessage::SystemStatsUpdate { cpu, gpu, ram, vram, network } => {
                self.system_stats = Some(SystemStats { cpu, gpu, ram, vram, network });
                self.dirty = true;
            }
        }
    }

    /// Mark UI as needing redraw.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Check and clear dirty flag.
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }
}
