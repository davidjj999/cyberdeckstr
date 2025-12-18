use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nostr_sdk::prelude::*;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, Paragraph},
    symbols,
    Frame, Terminal,
};
use serde::Deserialize;
use std::{io, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use bitcoincore_rpc::{Auth, Client as BtcClient, RpcApi};

const CYBER_GREEN: Color = Color::Rgb(0, 255, 65);
const CYBER_PINK: Color = Color::Rgb(255, 0, 255);
const CYBER_CYAN: Color = Color::Rgb(0, 240, 255);
const CYBER_BLACK: Color = Color::Rgb(10, 10, 16); // Very dark, almost black

enum AppState {
    Login,
    Connecting,
    Feed,
}

struct App {
    state: AppState,
    input: String, // For npub login
    messages: Vec<String>, // Simplified messages for display
    status: String,
    scroll: usize,
    client: Option<Client>,
    btc_history: Vec<(f64, f64)>,
    // Blockchain Viz State
    bitcoin_node_configured: bool,
    node_status: String,
    blocks: Vec<BlockDisplayInfo>,
    mempool: MempoolDisplayInfo,
    fees: FeeDisplayInfo,
}

#[derive(Clone)]
struct BlockDisplayInfo {
    height: u64,
    size: usize,
    timestamp: u64,
    tx_count: usize,
}

#[derive(Clone)]
struct MempoolDisplayInfo {
    size: usize,      // tx count
    usage: usize,     // memory usage
    max_mempool: usize, 
}

impl Default for MempoolDisplayInfo {
    fn default() -> Self {
        MempoolDisplayInfo { size: 0, usage: 0, max_mempool: 300_000_000 }
    }
}

#[derive(Clone)]
struct FeeDisplayInfo {
    low: f64,    // sat/vb
    medium: f64,
    high: f64,
}

impl Default for FeeDisplayInfo {
    fn default() -> Self {
        FeeDisplayInfo { low: 0.0, medium: 0.0, high: 0.0 }
    }
}

#[derive(Deserialize)]
struct MarketChart {
    prices: Vec<(f64, f64)>,
}

#[derive(Deserialize)]
struct Config {
    npub: Option<String>,
    node_address: Option<String>,
    node_username: Option<String>,
    node_password: Option<String>,
}

impl App {
    fn new() -> App {
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

#[tokio::main]
async fn main() -> Result<()> {
    // TUI setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create App state
    let mut app_state = App::new();

    // Try to load config
    let mut node_config = (None, None, None); // url, user, pass

    if let Ok(config_str) = std::fs::read_to_string("config.toml") {
        if let Ok(config) = toml::from_str::<Config>(&config_str) {
            if let Some(npub) = config.npub {
                 if !npub.is_empty() {
                     app_state.input = npub.clone();
                     app_state.state = AppState::Connecting;
                     app_state.status = "Initializing Uplink...".to_string();
                 }
            }
            // Check for node config
            if let (Some(addr), Some(user), Some(pass)) = (config.node_address, config.node_username, config.node_password) {
                if !addr.is_empty() && !user.is_empty() && !pass.is_empty() {
                    app_state.bitcoin_node_configured = true;
                    app_state.node_status = "CONNECTING TO NODE...".to_string();
                    node_config = (Some(addr), Some(user), Some(pass));
                }
            }
        }
    }

    let app = Arc::new(Mutex::new(app_state));
    
    // Spawn Blockchain Data Fetcher if configured
    if let (Some(url), Some(user), Some(pass)) = node_config {
        let app_btc_chain = app.clone();
        tokio::spawn(async move {
            fetch_blockchain_data(app_btc_chain, url, user, pass).await;
        });
    }



    // Channel for conveying events from the Nostr client task to the UI
    let (tx, rx) = mpsc::channel::<String>(100);

    // Auto-connect if configured
    {
        let guard = app.lock().await;
        if let AppState::Connecting = guard.state {
             let npub = guard.input.clone();
             let app_clone = app.clone();
             let tx_clone = tx.clone();
             tokio::spawn(async move {
                 if let Err(_e) = connect_nostr(npub, app_clone, tx_clone).await {
                 }
             });
        }
    }

    // Spawn BTC data fetcher
    let app_btc = app.clone();
    tokio::spawn(async move {
        fetch_btc_data(app_btc).await;
    });


    // Main loop
    let res = run_app(&mut terminal, app.clone(), tx, rx).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: Arc<Mutex<App>>,
    tx: mpsc::Sender<String>,
    mut rx: mpsc::Receiver<String>,
) -> Result<()> {
    // Input polling interval
    let mut interval = tokio::time::interval(Duration::from_millis(100));

    loop {
        {
            let app_guard = app.lock().await;
            terminal.draw(|f| ui(f, &app_guard))?;
        }

        tokio::select! {
             _ = interval.tick() => {
                 // Check for terminal events
                 if event::poll(Duration::from_millis(0))? {
                     if let Event::Key(key) = event::read()? {
                         let mut app_guard = app.lock().await;
                         match app_guard.state {
                             AppState::Login => {
                                 match key.code {
                                     KeyCode::Enter => {
                                         let npub = app_guard.input.clone();
                                         app_guard.state = AppState::Connecting;
                                         app_guard.status = "Initializing Uplink...".to_string();
                                         
                                         // Spawn connection task
                                         let app_clone = app.clone();
                                         let tx_clone = tx.clone();
                                         tokio::spawn(async move {
                                             if let Err(_e) = connect_nostr(npub, app_clone, tx_clone).await {
                                                 // Log error to console - this might mess up TUI if not careful
                                                 // But we are in a spawned task.
                                             }
                                         });
                                     }
                                     KeyCode::Char(c) => {
                                         app_guard.input.push(c);
                                     }
                                     KeyCode::Backspace => {
                                         app_guard.input.pop();
                                     }
                                     KeyCode::Esc => {
                                         return Ok(());
                                     }
                                     _ => {}
                                 }
                             }
                             AppState::Connecting => {
                                 if key.code == KeyCode::Esc {
                                      return Ok(());
                                 }
                             }
                             AppState::Feed => {
                                 match key.code {
                                     KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                                     KeyCode::Down => {
                                          if !app_guard.messages.is_empty() {
                                              if app_guard.scroll < app_guard.messages.len() - 1 {
                                                  app_guard.scroll += 1;
                                              }
                                          }
                                     }
                                     KeyCode::Up => {
                                         if app_guard.scroll > 0 {
                                             app_guard.scroll -= 1;
                                         }
                                     }
                                     _ => {}
                                 }
                             }
                         }
                     }
                 }
             }
             Some(msg) = rx.recv() => {
                 let mut app_guard = app.lock().await;
                 app_guard.messages.push(msg);
                 // Auto-scroll if at bottom
                 if app_guard.messages.len() > 0 {
                      app_guard.scroll = app_guard.messages.len() - 1;
                 }
             }
        }
    }
}

async fn connect_nostr(npub_str: String, app: Arc<Mutex<App>>, tx: mpsc::Sender<String>) -> Result<()> {
    // 1. Parse keys
    let public_key = match PublicKey::parse(&npub_str) {
        Ok(pk) => pk,
        Err(_) => {
            let mut app_guard = app.lock().await;
            app_guard.status = "INVALID IDENTITY. RETRY.".to_string();
            app_guard.state = AppState::Login;
            app_guard.input.clear();
            return Ok(());
        }
    };

    // 2. Create client (Ephemeral / Read Only)
    let client = Client::default();
    
    // 3. Add relays
    client.add_relay("wss://relay.damus.io").await?;
    client.add_relay("wss://nos.lol").await?;
    
    // 4. Connect
    {
        let mut app_guard = app.lock().await;
        app_guard.status = "JACKING IN...".to_string();
    }
    client.connect().await;

    // 5. Subscribe
    // Fetch contact list:
    let filter = Filter::new().kind(Kind::ContactList).author(public_key).limit(1);
    let events = client.fetch_events(filter, Duration::from_secs(5)).await.unwrap_or_default();
    
    let mut follows = Vec::new();
    if let Some(contact_list) = events.first() {
        for tag in contact_list.tags.iter() {
             // Fallback to slice parsing
             // ["p", "hex_pubkey", ...]
             let s = tag.as_slice();
             if s.len() >= 2 && s[0] == "p" {
                 if let Ok(pk) = PublicKey::parse(&s[1]) {
                     follows.push(pk);
                 }
             }
        }
    }
    
    // Add self to follows to see own notes if any
    follows.push(public_key);

    // Fetch Metadata for follows to resolve names
    let mut author_map = std::collections::HashMap::new();
    {
        let mut app_guard = app.lock().await;
        app_guard.status = "RESOLVING IDENTITIES...".to_string();
    }
    
    // Chunking to avoid too huge filters if follows is large, but for now just one batch
    // Many relays reject > 1000 authors. Assuming small follow list for this demo.
    let metadata_filter = Filter::new().kind(Kind::Metadata).authors(follows.clone());
    let metadata_events = client.fetch_events(metadata_filter, Duration::from_secs(5)).await.unwrap_or_default();

    for event in metadata_events {
        // Parse metadata
        if let Ok(metadata) = Metadata::from_json(&event.content) {
            let name = metadata.display_name.or(metadata.name).unwrap_or_else(|| "Unknown".to_string());
            author_map.insert(event.pubkey, name);
        }
    }

    let subscription = Filter::new()
        .kind(Kind::TextNote)
        .authors(follows)
        .limit(20);
    
    // Subscribe takes a single filter in 0.44+? Or Vec?
    // Error said "expected Filter, found Vec". So passing single filter.
    client.subscribe(subscription, None).await?;

    // 6. Update state
    {
        let mut app_guard = app.lock().await;
        app_guard.state = AppState::Feed;
        app_guard.status = "CONNECTED. SIGNAL ACQUIRED.".to_string();
        app_guard.client = Some(client.clone());
    }

    // 7. Handle Notifications
    let mut notifications = client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if event.kind == Kind::TextNote {
                let content = clean_content(&event.content);
                // Get author display name
                let author_name = author_map.get(&event.pubkey).cloned().unwrap_or_else(|| {
                     // Initial chars of pubkey
                     let pk_str = event.pubkey.to_string();
                     format!("{}...", &pk_str[0..8])
                });
                
                let display = format!("@{}: {}", author_name, content); 
                tx.send(display).await.ok();
            }
        }
    }

    Ok(())
}

async fn fetch_btc_data(app: Arc<Mutex<App>>) {
    let client = reqwest::Client::new();
    loop {
         // Fetch last 24 hours (1 day)
         let url = "https://api.coingecko.com/api/v3/coins/bitcoin/market_chart?vs_currency=usd&days=1";
         match client.get(url).header("User-Agent", "cyberdeckstr/1.0").send().await {
             Ok(resp) => {
                 if let Ok(chart_data) = resp.json::<MarketChart>().await {
                     let mut app_guard = app.lock().await;
                     app_guard.btc_history = chart_data.prices;
                 }
             }
             Err(_e) => { 
                 // Silently fail request or log if we had logging
             }
         }
         // Update every 60 seconds
         tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn clean_content(input: &str) -> String {
    input.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

async fn fetch_blockchain_data(app: Arc<Mutex<App>>, url: String, user: String, pass: String) {
    // bitcoincore-rpc is blocking, so we use spawn_blocking inside the loop or just run in this task?
    // Since this is a tokio::spawn'd task, we shouldn't block the thread.
    // Ideally we wrap the RPC calls.
    
    // Construct the full URL with auth if not present in string
    // bitcoincore-rpc expects "http://host:port" and separate Auth
    let rpc_url = if !url.starts_with("http") {
        format!("http://{}", url)
    } else {
        url
    };

    let auth = Auth::UserPass(user, pass);
    
    // Attempt connection loop
    loop {
        // We create client each time or reuse? Client is cheap?
        // Reuse client is better but need to handle potential disconnects?
        // bitcoincore_rpc::Client doesn't hold a persistent connection usually, it's HTTP.
        
        let client_url = rpc_url.clone();
        let client_auth = auth.clone();

        // Perform blocking RPC fetch
        let res = tokio::task::spawn_blocking(move || -> Result<(Vec<BlockDisplayInfo>, MempoolDisplayInfo, FeeDisplayInfo), String> {
             let client = BtcClient::new(&client_url, client_auth).map_err(|e| e.to_string())?;
             
             // 1. Get Blockchain Info / Best Block
             let blockchain_info = client.get_blockchain_info().map_err(|e| e.to_string())?;
             let best_height = blockchain_info.blocks;
             
             // 2. Fetch last 6 blocks
             let mut blocks = Vec::new();
             let mut current_hash = blockchain_info.best_block_hash;
             
             for _ in 0..6 {
                 let block = client.get_block(&current_hash).map_err(|e| e.to_string())?;
                 blocks.push(BlockDisplayInfo {
                     height: 0, // We don't get height directly from get_block easily without header? 
                     // Wait, get_block returns Block which has header. Header doesn't have height.
                     // But we know best_height.
                     // It's simpler to fetch block hash by height?
                     // Let's use get_block_hash(height)
                     size: block.total_size(),
                     timestamp: block.header.time as u64,
                     tx_count: block.txdata.len(),
                 });
                 current_hash = block.header.prev_blockhash;
             }
             
             // Fix heights (since we went backwards)
             for (i, b) in blocks.iter_mut().enumerate() {
                 b.height = best_height - i as u64;
             }
             
             // 3. Mempool Info
             let mempool_info = client.get_mempool_info().map_err(|e| e.to_string())?;
             let mempool = MempoolDisplayInfo {
                 size: mempool_info.size,
                 usage: mempool_info.usage,
                 max_mempool: mempool_info.max_mempool,
             };
             
             // 4. Smart Fee
             
             let f_low = client.estimate_smart_fee(144, None).ok().and_then(|r| r.fee_rate).map(|a| a.to_btc() * 100_000.0).unwrap_or(0.0);
             let f_med = client.estimate_smart_fee(6, None).ok().and_then(|r| r.fee_rate).map(|a| a.to_btc() * 100_000.0).unwrap_or(0.0);
             let f_high = client.estimate_smart_fee(1, None).ok().and_then(|r| r.fee_rate).map(|a| a.to_btc() * 100_000.0).unwrap_or(0.0);

             let fees = FeeDisplayInfo {
                 low: f_low,
                 medium: f_med,
                 high: f_high,
             };
             
             Ok((blocks, mempool, fees))
        }).await;
        
        match res {
            Ok(Ok((blocks, mempool, fees))) => {
                 let mut app_guard = app.lock().await;
                 app_guard.blocks = blocks;
                 app_guard.mempool = mempool;
                 app_guard.fees = fees;
                 app_guard.node_status = "NODE CONNECTED".to_string();
            }
            Ok(Err(e)) => {
                 let mut app_guard = app.lock().await;
                 app_guard.node_status = format!("NODE ERROR: {}", e);
            }
            Err(e) => {
                 let mut app_guard = app.lock().await;
                 app_guard.node_status = format!("TASK ERROR: {}", e);
            }
        }
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();
    
    // Cyberpunk Style
    let border_style = Style::default().fg(CYBER_PINK);
    let text_style = Style::default().fg(CYBER_GREEN).bg(CYBER_BLACK);
    let highlight_style = Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD);

    let constraints = if app.bitcoin_node_configured {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(10), // BTC Chart (Small)
            Constraint::Length(14), // Viz
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Status
        ]
    } else {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(12), // BTC Chart
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Status
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(size);
    
    // Header
    let header_text = match app.state {
         AppState::Login => "AUTH SEQUENCE",
         AppState::Connecting => "ESTABLISHING UPLINK",
         AppState::Feed => "DATA STREAM",
    };
    
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" CYBERDECKSTR 1.0 "));
    f.render_widget(header, chunks[0]);

    // Helper to render Chart
    let render_chart = |f: &mut Frame, area: ratatui::layout::Rect, app: &App| {
             let chart_block = Block::default()
            .title(" BTC/USD (24H) ")
            .borders(Borders::ALL)
            .border_style(border_style);
    
        let datasets = vec![
            Dataset::default()
                .name("Price")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(CYBER_GREEN))
                .graph_type(GraphType::Line)
                .data(&app.btc_history),
        ];
    
        // Calculate bounds
         let (min_x, max_x, min_y, max_y) = if app.btc_history.is_empty() {
            (0.0, 100.0, 0.0, 100.0)
        } else {
            let xs: Vec<f64> = app.btc_history.iter().map(|(x, _)| *x).collect();
            let ys: Vec<f64> = app.btc_history.iter().map(|(_, y)| *y).collect();
            (
                xs.first().cloned().unwrap_or(0.0),
                xs.last().cloned().unwrap_or(100.0),
                ys.iter().cloned().fold(f64::INFINITY, f64::min),
                ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            )
        };
    
        let chart = Chart::new(datasets)
            .block(chart_block)
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_x, max_x])
                    .labels(vec![
                        Span::styled(" -24h ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(" Now ", Style::default().add_modifier(Modifier::BOLD)),
                    ]),
            )
            .y_axis(
                Axis::default()
                    .title("Price")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_y, max_y])
                    .labels(vec![
                        Span::styled(format!("{:.0}", min_y), Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:.0}", max_y), Style::default().add_modifier(Modifier::BOLD)),
                    ]),
            );
        f.render_widget(chart, area);
    };

    // Visualization Area & Chart
    if app.bitcoin_node_configured {
        // 1. BTC Chart
        render_chart(f, chunks[1], app);
        
        // 2. Blockchain Viz
        let viz_area = chunks[2];
        let vz_block = Block::default().borders(Borders::ALL).border_style(border_style).title(" BITCOIN MAINNET ");
        f.render_widget(vz_block, viz_area);
        
        let inner_area = viz_area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 1 });
        let viz_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // Blocks row
                Constraint::Min(0),    // Rest
            ].as_ref())
            .split(inner_area);
            
        // Render Blocks
        let block_constraints = vec![Constraint::Ratio(1, 6); 6];
        let block_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(block_constraints)
            .split(viz_rows[0]);
            
        for (i, block) in app.blocks.iter().enumerate() {
             if i >= 6 { break; }
             
             // Time diff
             let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
             let diff = now.saturating_sub(block.timestamp);
             let time_str = if diff < 60 { format!("{}s", diff) } else { format!("{}m", diff / 60) };
             
             let b_text = format!(
                 "{}\n{} txs\n{}\n{:.2}MB",
                 block.height,
                 block.tx_count,
                 time_str,
                 block.size as f64 / 1_000_000.0
             );
             
             let b_widget = Paragraph::new(b_text)
                .alignment(ratatui::layout::Alignment::Center)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(CYBER_GREEN)).title("BLK"));
             f.render_widget(b_widget, block_cols[i]);
        }
        
        // Bottom Row: Fees + Mempool
        let stat_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ].as_ref())
            .split(viz_rows[1]);
            
        // Fees
        let fee_text = format!(
            "LOW: {:.0} sat/vB\nMED: {:.0} sat/vB\nHIGH: {:.0} sat/vB",
            app.fees.low, app.fees.medium, app.fees.high
        );
        let fees_widget = Paragraph::new(fee_text)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(CYBER_PINK)).title(" FEES "))
            .style(text_style);
        f.render_widget(fees_widget, stat_cols[0]);
        
        // Mempool
        let mem_mb = app.mempool.usage as f64 / 1_000_000.0;
        let max_mb = app.mempool.max_mempool as f64 / 1_000_000.0;
        let ratio = (mem_mb / max_mb).clamp(0.0, 1.0);
        
        let mem_gauge = ratatui::widgets::Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" MEMPOOL RAM "))
            .gauge_style(Style::default().fg(CYBER_CYAN))
            .ratio(ratio)
            .label(format!("{:.1} / {:.1} MB ({} txs)", mem_mb, max_mb, app.mempool.size));
        f.render_widget(mem_gauge, stat_cols[1]);

    } else {
        // BTC Chart (Legacy behavior)
        render_chart(f, chunks[1], app);
    }

    // Content
    let content_chunk_index = if app.bitcoin_node_configured { 3 } else { 2 };
    
    match app.state {
        AppState::Login => {
            let input = Paragraph::new(app.input.as_str())
                .style(text_style)
                .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" ENTER NPUB IDENTITY "));
            f.render_widget(input, chunks[content_chunk_index]);
        }
        AppState::Connecting => {
             let loading = Paragraph::new("DECRYPTING REALITY...")
                .style(Style::default().fg(CYBER_GREEN).add_modifier(Modifier::RAPID_BLINK))
                .block(Block::default().borders(Borders::ALL).border_style(border_style));
             f.render_widget(loading, chunks[content_chunk_index]);
        }
        AppState::Feed => {
            // Calculate available width for text
            let max_width = chunks[content_chunk_index].width.saturating_sub(4) as usize;

            let messages: Vec<ListItem> = app.messages
                .iter()
                .map(|m| {
                    // Wrap the message
                    let wrapped_lines = textwrap::wrap(m, max_width);
                    
                    let mut lines = vec![Line::from(Span::raw(""))]; // Spacer
                    
                    for line in wrapped_lines {
                        lines.push(Line::from(Span::styled(line.to_string(), text_style)));
                    }
                    
                    lines.push(Line::from(Span::styled("---", Style::default().fg(Color::DarkGray))));
                    
                    ListItem::new(lines)
                })
                .collect();
            
            let messages_list = List::new(messages)
                .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" LIVE FEED "))
                .highlight_style(highlight_style); 

            let mut state = ratatui::widgets::ListState::default();
            state.select(Some(app.scroll));
            
            f.render_stateful_widget(messages_list, chunks[content_chunk_index], &mut state);
        }
    }

    // Footer / Status
    let status_chunk_index = if app.bitcoin_node_configured { 4 } else { 3 };
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::White).bg(Color::Rgb(20, 20, 30)))
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" SYSTEM STATUS "));
    f.render_widget(status, chunks[status_chunk_index]);
}
