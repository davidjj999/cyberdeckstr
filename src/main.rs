mod app;
mod config;
mod network;
mod ui;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use app::{App, AppState};
use config::Config;
use network::{connect_nostr, fetch_blockchain_data, fetch_btc_data};
use ui::ui;

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
    let (tx, rx) = mpsc::channel::<(String, String)>(100);

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
    tx: mpsc::Sender<(String, String)>,
    mut rx: mpsc::Receiver<(String, String)>,
) -> Result<()> {
    // Adaptive polling: start with normal interval, slow down when idle
    const ACTIVE_POLL_MS: u64 = 50;    // 20fps when active
    const IDLE_POLL_MS: u64 = 200;     // 5fps when idle
    const IDLE_THRESHOLD_MS: u64 = 5000; // Consider idle after 5 seconds of no activity

    let mut last_activity = std::time::Instant::now();
    let mut last_render = std::time::Instant::now();
    
    // Minimum render interval to cap at ~30fps
    let min_render_interval = Duration::from_millis(33);

    loop {
        // Calculate current poll timeout based on activity
        let idle_duration = last_activity.elapsed().as_millis() as u64;
        let poll_timeout = if idle_duration > IDLE_THRESHOLD_MS {
            Duration::from_millis(IDLE_POLL_MS)
        } else {
            Duration::from_millis(ACTIVE_POLL_MS)
        };

        tokio::select! {
            // Handle terminal input events with adaptive timeout
            _ = tokio::time::sleep(poll_timeout) => {
                // Check for terminal events with non-blocking poll
                if event::poll(Duration::from_millis(0))? {
                    if let Event::Key(key) = event::read()? {
                        last_activity = std::time::Instant::now();
                        let mut app_guard = app.lock().await;
                        app_guard.mark_dirty();
                        
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
                                                // Error handled inside connect_nostr
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
            
            // Handle incoming Nostr messages
            Some((id, msg)) = rx.recv() => {
                last_activity = std::time::Instant::now();
                let mut app_guard = app.lock().await;
                app_guard.add_message(id, msg);
            }
        }

        // Event-driven rendering: only render when dirty and enough time has passed
        let should_render = {
            let mut app_guard = app.lock().await;
            let has_passed = last_render.elapsed() >= min_render_interval;
            if has_passed && app_guard.consume_dirty() {
                true
            } else {
                false
            }
        };

        if should_render {
            let app_guard = app.lock().await;
            terminal.draw(|f| ui(f, &app_guard))?;
            last_render = std::time::Instant::now();
        }
    }
}

