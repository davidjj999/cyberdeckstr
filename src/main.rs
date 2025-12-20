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
