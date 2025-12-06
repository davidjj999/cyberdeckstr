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
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

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
    let app = Arc::new(Mutex::new(App::new()));

    // Channel for conveying events from the Nostr client task to the UI
    let (tx, rx) = mpsc::channel::<String>(100);

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
                let content = event.content.clone();
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

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();
    
    // Cyberpunk Style
    let border_style = Style::default().fg(CYBER_PINK);
    let text_style = Style::default().fg(CYBER_GREEN).bg(CYBER_BLACK);
    let highlight_style = Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Content
                Constraint::Length(3), // Status
            ]
            .as_ref(),
        )
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

    // Content
    match app.state {
        AppState::Login => {
            let input = Paragraph::new(app.input.as_str())
                .style(text_style)
                .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" ENTER NPUB IDENTITY "));
            f.render_widget(input, chunks[1]);
        }
        AppState::Connecting => {
             let loading = Paragraph::new("DECRYPTING REALITY...")
                .style(Style::default().fg(CYBER_GREEN).add_modifier(Modifier::RAPID_BLINK))
                .block(Block::default().borders(Borders::ALL).border_style(border_style));
             f.render_widget(loading, chunks[1]);
        }
        AppState::Feed => {
            // Calculate available width for text
            // Chunks[1] width - 2 (borders) - 2 (left/right padding if any, let's say 2 for safety)
            let max_width = chunks[1].width.saturating_sub(4) as usize;

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
            
            f.render_stateful_widget(messages_list, chunks[1], &mut state);
        }
    }

    // Footer / Status
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::White).bg(Color::Rgb(20, 20, 30)))
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" SYSTEM STATUS "));
    f.render_widget(status, chunks[2]);
}
