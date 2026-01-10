use crate::app::{AppState, SharedApp, MarketChart, BlockDisplayInfo, MempoolDisplayInfo, FeeDisplayInfo};
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;
use tokio::sync::mpsc;
use bitcoincore_rpc::{Auth, Client as BtcClient, RpcApi};

pub async fn connect_nostr(npub_str: String, app: SharedApp, tx: mpsc::Sender<(String, String)>) -> Result<()> {
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
    
    // 3. Add bootstrap relays
    client.add_relay("wss://relay.damus.io").await?;
    client.add_relay("wss://nos.lol").await?;
    client.add_relay("wss://relay.primal.net").await?;
    
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

    // --- RELAY OPTIMIZATION ---
    if !follows.is_empty() {
        {
            let mut app_guard = app.lock().await;
            app_guard.status = "DISCOVERING RELAYS...".to_string();
        }
        
        // Fetch RealmList (10002) for all follows
        // Splitting into chunks to avoid potential filter limits, though fetching 10002 usually is fast.
        let relay_filter = Filter::new().kind(Kind::RelayList).authors(follows.clone());
        let relay_events = client.fetch_events(relay_filter, Duration::from_secs(6)).await.unwrap_or_default();
        
        let mut relay_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        
        for event in relay_events {
             for tag in event.tags.iter() {
                 let s = tag.as_slice();
                 if s.len() >= 2 && s[0] == "r" {
                     // Normalize URL lightly?
                     let url = s[1].trim_end_matches('/').to_string();
                     *relay_counts.entry(url).or_insert(0) += 1;
                 }
             }
        }
        
        // Ranking
        let mut ranked_relays: Vec<(String, usize)> = relay_counts.into_iter().collect();
        ranked_relays.sort_by(|a, b| b.1.cmp(&a.1)); // Descending count
        
        let top_k = 5;
        let top_relays: Vec<String> = ranked_relays.iter().take(top_k).map(|(r, _)| r.clone()).collect();
        
        if !top_relays.is_empty() {
             {
                let mut app_guard = app.lock().await;
                app_guard.status = format!("OPTIMIZING UPLINK ({} RELAYS)...", top_relays.len());
             }
             
             for url in top_relays {
                 // Add relay if not exists
                 client.add_relay(url).await.ok(); 
             }
             
             // Re-connect to ensure new relays are active
             client.connect().await;
             
             // Give it a moment to stabilize
             tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }
    // --------------------------

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
                tx.send((event.id.to_string(), display)).await.ok();
            }
        }
    }

    Ok(())
}

pub async fn fetch_btc_data(app: SharedApp) {
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

pub fn clean_content(input: &str) -> String {
    input.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

pub async fn fetch_blockchain_data(app: SharedApp, url: String, user: String, pass: String) {
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
