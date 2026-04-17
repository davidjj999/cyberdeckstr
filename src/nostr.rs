use crate::app::AppMessage;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

/// Top-level Nostr connection lifecycle.
///
/// Parses the npub, bootstraps relays, discovers the follow graph,
/// optimises relay connections, resolves display names, subscribes
/// to the feed, and runs the notification loop with health-checks.
///
/// All state mutations are communicated back to the main loop via `tx`.
pub async fn connect_nostr(npub_str: String, tx: mpsc::Sender<AppMessage>) {
    if let Err(e) = connect_nostr_inner(&npub_str, &tx).await {
        tracing::error!("Nostr connection failed: {}", e);
        let _ = tx.send(AppMessage::NostrLoginError(
            format!("CONNECTION ERROR: {}", e),
        )).await;
    }
}

async fn connect_nostr_inner(npub_str: &str, tx: &mpsc::Sender<AppMessage>) -> Result<()> {
    let public_key = match parse_identity(npub_str) {
        Some(pk) => pk,
        None => {
            let _ = tx.send(AppMessage::NostrLoginError(
                "INVALID IDENTITY. RETRY.".to_string(),
            )).await;
            return Ok(());
        }
    };

    let client = create_client().await?;

    let _ = tx.send(AppMessage::NostrStatus("JACKING IN...".to_string())).await;
    client.connect().await;

    let follows = discover_follows(&client, public_key).await;
    optimize_relays(&client, &follows, tx).await;
    let author_map = resolve_metadata(&client, &follows, tx).await;

    // Subscribe to text notes from the follow list
    let subscription = Filter::new()
        .kind(Kind::TextNote)
        .authors(follows)
        .limit(20);
    client.subscribe(subscription, None).await?;

    let _ = tx.send(AppMessage::NostrConnected).await;
    tracing::info!("Nostr feed subscription active");

    run_event_loop(client, author_map, tx).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Decomposed helpers
// ---------------------------------------------------------------------------

fn parse_identity(npub_str: &str) -> Option<PublicKey> {
    match PublicKey::parse(npub_str) {
        Ok(pk) => Some(pk),
        Err(e) => {
            tracing::warn!("Failed to parse npub: {}", e);
            None
        }
    }
}

async fn create_client() -> Result<Client> {
    let client = Client::default();

    // Bootstrap relays
    client.add_relay("wss://relay.damus.io").await?;
    client.add_relay("wss://nos.lol").await?;
    client.add_relay("wss://relay.primal.net").await?;

    Ok(client)
}

/// Fetch the contact list (Kind 3) and return the set of followed pubkeys.
async fn discover_follows(client: &Client, public_key: PublicKey) -> Vec<PublicKey> {
    let filter = Filter::new()
        .kind(Kind::ContactList)
        .author(public_key)
        .limit(1);

    let events = match client.fetch_events(filter, Duration::from_secs(5)).await {
        Ok(evs) => evs,
        Err(e) => {
            tracing::warn!("Failed to fetch contact list: {}", e);
            return vec![public_key];
        }
    };

    let mut follows = Vec::new();
    if let Some(contact_list) = events.first() {
        for tag in contact_list.tags.iter() {
            let s = tag.as_slice();
            if s.len() >= 2 && s[0] == "p" {
                if let Ok(pk) = PublicKey::parse(&s[1]) {
                    follows.push(pk);
                }
            }
        }
    }

    // Include self to see own notes
    follows.push(public_key);
    tracing::info!("Discovered {} follows", follows.len());
    follows
}

/// Fetch relay lists (Kind 10002) for follows, rank by frequency, and add the
/// top relays to the client.
async fn optimize_relays(
    client: &Client,
    follows: &[PublicKey],
    tx: &mpsc::Sender<AppMessage>,
) {
    if follows.is_empty() {
        return;
    }

    let _ = tx.send(AppMessage::NostrStatus("DISCOVERING RELAYS...".to_string())).await;

    let relay_filter = Filter::new()
        .kind(Kind::RelayList)
        .authors(follows.to_vec());

    let relay_events = match client.fetch_events(relay_filter, Duration::from_secs(6)).await {
        Ok(evs) => evs,
        Err(e) => {
            tracing::warn!("Failed to fetch relay lists: {}", e);
            return;
        }
    };

    let mut relay_counts: HashMap<String, usize> = HashMap::new();
    for event in relay_events {
        for tag in event.tags.iter() {
            let s = tag.as_slice();
            if s.len() >= 2 && s[0] == "r" {
                let url = s[1].trim_end_matches('/').to_string();
                *relay_counts.entry(url).or_insert(0) += 1;
            }
        }
    }

    // Rank by frequency, take top 5
    let mut ranked: Vec<(String, usize)> = relay_counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let top_relays: Vec<String> = ranked.iter().take(5).map(|(r, _)| r.clone()).collect();

    if top_relays.is_empty() {
        return;
    }

    let _ = tx.send(AppMessage::NostrStatus(
        format!("OPTIMIZING UPLINK ({} RELAYS)...", top_relays.len()),
    )).await;

    for url in &top_relays {
        if let Err(e) = client.add_relay(url).await {
            tracing::debug!("Couldn't add relay {}: {}", url, e);
        }
    }

    // Reconnect to activate new relays
    client.connect().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    tracing::info!("Relay optimization complete — {} relays added", top_relays.len());
}

/// Fetch Kind 0 metadata for all follows and build a pubkey → display name map.
async fn resolve_metadata(
    client: &Client,
    follows: &[PublicKey],
    tx: &mpsc::Sender<AppMessage>,
) -> HashMap<PublicKey, String> {
    let _ = tx.send(AppMessage::NostrStatus("RESOLVING IDENTITIES...".to_string())).await;

    let metadata_filter = Filter::new()
        .kind(Kind::Metadata)
        .authors(follows.to_vec());

    let metadata_events = match client.fetch_events(metadata_filter, Duration::from_secs(5)).await {
        Ok(evs) => evs,
        Err(e) => {
            tracing::warn!("Failed to fetch metadata: {}", e);
            return HashMap::new();
        }
    };

    let mut author_map = HashMap::new();
    for event in metadata_events {
        if let Ok(metadata) = Metadata::from_json(&event.content) {
            let name = metadata
                .display_name
                .or(metadata.name)
                .unwrap_or_else(|| "Unknown".to_string());
            author_map.insert(event.pubkey, name);
        }
    }

    tracing::info!("Resolved {} author identities", author_map.len());
    author_map
}

/// Long-running notification loop with periodic health checks for
/// stale connections (e.g. after system suspend/resume).
async fn run_event_loop(
    client: Client,
    author_map: HashMap<PublicKey, String>,
    tx: &mpsc::Sender<AppMessage>,
) {
    let mut notifications = client.notifications();

    const HEALTH_CHECK_INTERVAL_SECS: u64 = 120;
    const RECONNECT_TIMEOUT_SECS: u64 = 180;

    let mut last_data_time = std::time::Instant::now();
    let mut health_check_interval =
        tokio::time::interval(Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));
    health_check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = notifications.recv() => {
                match result {
                    Ok(notification) => {
                        last_data_time = std::time::Instant::now();

                        if let RelayPoolNotification::Event { event, .. } = notification {
                            if event.kind == Kind::TextNote {
                                let content = clean_content(&event.content);
                                let author_name = author_map
                                    .get(&event.pubkey)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        let pk_str = event.pubkey.to_string();
                                        format!("{}...", &pk_str[0..8])
                                    });

                                let display = format!("@{}: {}", author_name, content);
                                let _ = tx.send(AppMessage::NostrEvent {
                                    id: event.id,
                                    display,
                                }).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Notification channel error: {}", e);
                        let _ = tx.send(AppMessage::NostrStatus(
                            "CONNECTION LOST. RECONNECTING...".to_string(),
                        )).await;

                        reconnect(&client, &mut notifications).await;

                        let _ = tx.send(AppMessage::NostrStatus(
                            "RECONNECTED. SIGNAL ACQUIRED.".to_string(),
                        )).await;
                        last_data_time = std::time::Instant::now();
                    }
                }
            }

            _ = health_check_interval.tick() => {
                let elapsed = last_data_time.elapsed().as_secs();
                if elapsed > RECONNECT_TIMEOUT_SECS {
                    tracing::info!("Stale connection detected ({}s idle), reconnecting", elapsed);
                    let _ = tx.send(AppMessage::NostrStatus(
                        "STALE CONNECTION DETECTED. RECONNECTING...".to_string(),
                    )).await;

                    reconnect(&client, &mut notifications).await;

                    let _ = tx.send(AppMessage::NostrStatus(
                        "RECONNECTED. SIGNAL ACQUIRED.".to_string(),
                    )).await;
                    last_data_time = std::time::Instant::now();
                }
            }
        }
    }
}

/// Disconnect, wait briefly, reconnect, and re-acquire the notifications channel.
async fn reconnect(
    client: &Client,
    notifications: &mut tokio::sync::broadcast::Receiver<RelayPoolNotification>,
) {
    client.disconnect().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    client.connect().await;
    *notifications = client.notifications();
}

/// Strip control characters from note content, preserving newlines and tabs.
fn clean_content(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
