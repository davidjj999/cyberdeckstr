use nostr_sdk::prelude::*;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, PartialEq)]
pub enum AppState {
    Login,
    Connecting,
    Feed,
}

pub struct App {
    pub state: AppState,
    pub input: String, // For npub login
    pub messages: Vec<String>, // Simplified messages for display
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
            messages: Vec::new(),
            status: "Welcome to CYBERDECKSTR. Enter NPUB.".to_string(),
            scroll: 0,
            client: None,
            btc_history: Vec::new(),
            bitcoin_node_configured: false,
            node_status: "Node Disconnected".to_string(),
            blocks: Vec::new(),
            mempool: MempoolDisplayInfo::default(),
            fees: FeeDisplayInfo::default(),
        }
    }
}

pub type SharedApp = Arc<Mutex<App>>;
