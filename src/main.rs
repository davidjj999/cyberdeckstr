mod app;
mod bitcoin;
mod config;
mod market;
mod nostr;
mod system_stats;
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
use std::{io, time::Duration};
use tokio::sync::mpsc;

use app::{App, AppMessage, AppState, NodeState};
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // -----------------------------------------------------------------------
    // 1. Structured logging to file (safe for TUI — never writes to stdout)
    // -----------------------------------------------------------------------
    let file_appender = tracing_appender::rolling::daily("logs", "cyberdeckstr.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    tracing::info!("CYBERDECKSTR starting up");

    // -----------------------------------------------------------------------
    // 2. Panic hook — restore terminal before printing backtrace
    // -----------------------------------------------------------------------
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    // -----------------------------------------------------------------------
    // 3. TUI setup
    // -----------------------------------------------------------------------
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // -----------------------------------------------------------------------
    // 4. App state — owned exclusively by the main loop (no Arc<Mutex>)
    // -----------------------------------------------------------------------
    let mut app = App::new();

    // Single channel: all background tasks send AppMessage here
    let (tx, rx) = mpsc::channel::<AppMessage>(256);

    // -----------------------------------------------------------------------
    // 5. Load config and spawn background tasks
    // -----------------------------------------------------------------------
    if let Ok(config_str) = std::fs::read_to_string("config.toml") {
        match toml::from_str::<Config>(&config_str) {
            Ok(config) => {
                tracing::info!("Configuration loaded successfully");

                // Bitcoin node data fetcher
                if let (Some(addr), Some(user), Some(pass)) =
                    (config.node_address, config.node_username, config.node_password)
                {
                    if !addr.is_empty() && !user.is_empty() && !pass.is_empty() {
                        app.node = Some(NodeState::new());
                        let tx_btc = tx.clone();
                        tokio::spawn(async move {
                            bitcoin::fetch_blockchain_data(tx_btc, addr, user, pass).await;
                        });
                        tracing::info!("Bitcoin node data fetcher spawned");
                    }
                }

                // Auto-connect Nostr if npub is configured
                if let Some(npub) = config.npub {
                    if !npub.is_empty() {
                        app.input = npub.clone();
                        app.state = AppState::Connecting;
                        app.status = "Initializing Uplink...".to_string();

                        let tx_nostr = tx.clone();
                        tokio::spawn(async move {
                            nostr::connect_nostr(npub, tx_nostr).await;
                        });
                        tracing::info!("Nostr auto-connect spawned");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to parse config.toml: {}", e);
            }
        }
    } else {
        tracing::info!("No config.toml found, starting in login mode");
    }

    // BTC market price fetcher (always runs)
    let tx_market = tx.clone();
    tokio::spawn(async move {
        market::fetch_btc_data(tx_market).await;
    });

    // System monitor stats fetcher (always runs)
    let tx_sys = tx.clone();
    tokio::spawn(async move {
        system_stats::monitor_system_stats(tx_sys).await;
    });

    // -----------------------------------------------------------------------
    // 6. Main event loop
    // -----------------------------------------------------------------------
    let res = run_app(&mut terminal, &mut app, tx, rx).await;

    // -----------------------------------------------------------------------
    // 7. Restore terminal
    // -----------------------------------------------------------------------
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

    tracing::info!("CYBERDECKSTR shutdown complete");
    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    tx: mpsc::Sender<AppMessage>,
    mut rx: mpsc::Receiver<AppMessage>,
) -> Result<()> {
    // Adaptive polling: fast when active, slow when idle
    const ACTIVE_POLL_MS: u64 = 50;     // 20fps
    const IDLE_POLL_MS: u64 = 200;      // 5fps
    const IDLE_THRESHOLD_MS: u64 = 5000; // idle after 5s of no activity

    let mut last_activity = std::time::Instant::now();
    let mut last_render = std::time::Instant::now();
    let min_render_interval = Duration::from_millis(33); // ~30fps cap

    loop {
        let idle_duration = last_activity.elapsed().as_millis() as u64;
        let poll_timeout = if idle_duration > IDLE_THRESHOLD_MS {
            Duration::from_millis(IDLE_POLL_MS)
        } else {
            Duration::from_millis(ACTIVE_POLL_MS)
        };

        tokio::select! {
            // Handle terminal input events with adaptive timeout
            _ = tokio::time::sleep(poll_timeout) => {
                if event::poll(Duration::from_millis(0))? {
                    match event::read()? {
                        Event::Key(key) => {
                            last_activity = std::time::Instant::now();
                            app.mark_dirty();

                            match app.state {
                                AppState::Login => {
                                    match key.code {
                                        KeyCode::Enter => {
                                            let npub = app.input.clone();
                                            app.state = AppState::Connecting;
                                            app.status = "Initializing Uplink...".to_string();

                                            let tx_clone = tx.clone();
                                            tokio::spawn(async move {
                                                nostr::connect_nostr(npub, tx_clone).await;
                                            });
                                        }
                                        KeyCode::Char(c) => {
                                            app.input.push(c);
                                        }
                                        KeyCode::Backspace => {
                                            app.input.pop();
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
                                            if !app.feed.entries.is_empty()
                                                && app.scroll < app.feed.entries.len() - 1
                                            {
                                                app.scroll += 1;
                                            }
                                        }
                                        KeyCode::Up => {
                                            if app.scroll > 0 {
                                                app.scroll -= 1;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        Event::Resize(_, _) => {
                            app.mark_dirty();
                        }
                        _ => {}
                    }
                }
            }

            // Handle messages from background tasks (lock-free!)
            Some(msg) = rx.recv() => {
                last_activity = std::time::Instant::now();
                app.handle_message(msg);
            }
        }

        // Event-driven rendering: only when dirty and frame budget allows
        if last_render.elapsed() >= min_render_interval && app.consume_dirty() {
            // Check for terminal resize → invalidate text-wrap cache
            let content_width = terminal.size()?.width.saturating_sub(8) as usize;
            if content_width > 0 && content_width != app.feed.cached_wrap_width {
                app.feed.rewrap(content_width);
            }

            terminal.draw(|f| ui::ui(f, app))?;
            last_render = std::time::Instant::now();
        }
    }
}
