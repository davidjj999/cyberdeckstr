use crate::app::{AppMessage, BlockDisplayInfo, FeeDisplayInfo, MempoolDisplayInfo};
use bitcoincore_rpc::{Auth, Client as BtcClient, RpcApi};
use std::time::Duration;
use tokio::sync::mpsc;

/// Conversion factor: BTC/kB → sat/vB.
const BTC_PER_KB_TO_SAT_PER_VB: f64 = 100_000.0;

/// Long-running task that polls a Bitcoin Core node for blockchain data.
///
/// Uses exponential backoff on failure (5s → 60s cap).
pub async fn fetch_blockchain_data(
    tx: mpsc::Sender<AppMessage>,
    url: String,
    user: String,
    pass: String,
) {
    let rpc_url = if !url.starts_with("http") {
        format!("http://{}", url)
    } else {
        url
    };

    let auth = Auth::UserPass(user, pass);

    // Exponential backoff parameters
    const BASE_DELAY_SECS: u64 = 5;
    const MAX_DELAY_SECS: u64 = 60;
    let mut consecutive_failures: u32 = 0;

    loop {
        let client_url = rpc_url.clone();
        let client_auth = auth.clone();

        // bitcoincore-rpc is blocking — run on the blocking threadpool
        let res = tokio::task::spawn_blocking(move || fetch_from_node(&client_url, client_auth)).await;

        let delay_secs = match res {
            Ok(Ok((blocks, mempool, fees))) => {
                let _ = tx.send(AppMessage::BlockchainUpdate { blocks, mempool, fees }).await;
                consecutive_failures = 0;
                BASE_DELAY_SECS
            }
            Ok(Err(e)) => {
                tracing::warn!("Bitcoin RPC error: {}", e);
                let _ = tx.send(AppMessage::BlockchainStatus(
                    format!("NODE ERROR: {}", e),
                )).await;
                consecutive_failures = consecutive_failures.saturating_add(1);
                backoff_delay(BASE_DELAY_SECS, MAX_DELAY_SECS, consecutive_failures)
            }
            Err(e) => {
                tracing::error!("Bitcoin RPC task panicked: {}", e);
                let _ = tx.send(AppMessage::BlockchainStatus(
                    format!("TASK ERROR: {}", e),
                )).await;
                consecutive_failures = consecutive_failures.saturating_add(1);
                backoff_delay(BASE_DELAY_SECS, MAX_DELAY_SECS, consecutive_failures)
            }
        };

        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
    }
}

/// Perform all blocking RPC calls inside `spawn_blocking`.
fn fetch_from_node(
    url: &str,
    auth: Auth,
) -> Result<(Vec<BlockDisplayInfo>, MempoolDisplayInfo, FeeDisplayInfo), String> {
    let client = BtcClient::new(url, auth).map_err(|e| e.to_string())?;

    // 1. Blockchain info
    let blockchain_info = client.get_blockchain_info().map_err(|e| e.to_string())?;
    let best_height = blockchain_info.blocks;

    // 2. Last 6 blocks (walking backwards via prev_blockhash)
    let mut blocks = Vec::with_capacity(6);
    let mut current_hash = blockchain_info.best_block_hash;

    for _ in 0..6 {
        let block = client.get_block(&current_hash).map_err(|e| e.to_string())?;
        blocks.push(BlockDisplayInfo {
            height: 0, // fixed up below
            size: block.total_size(),
            timestamp: block.header.time as u64,
            tx_count: block.txdata.len(),
        });
        current_hash = block.header.prev_blockhash;
    }

    // Fix heights (we walked backwards from best)
    for (i, b) in blocks.iter_mut().enumerate() {
        b.height = best_height - i as u64;
    }

    // 3. Mempool info
    let mempool_info = client.get_mempool_info().map_err(|e| e.to_string())?;
    let mempool = MempoolDisplayInfo {
        size: mempool_info.size,
        usage: mempool_info.usage,
        max_mempool: mempool_info.max_mempool,
    };

    // 4. Fee estimates (low=144 blocks, medium=6, high=1)
    let f_low = estimate_fee(&client, 144);
    let f_med = estimate_fee(&client, 6);
    let f_high = estimate_fee(&client, 1);

    let fees = FeeDisplayInfo {
        low: f_low,
        medium: f_med,
        high: f_high,
    };

    Ok((blocks, mempool, fees))
}

/// Estimate fee in sat/vB for a given confirmation target.
fn estimate_fee(client: &BtcClient, conf_target: u16) -> f64 {
    client
        .estimate_smart_fee(conf_target, None)
        .ok()
        .and_then(|r| r.fee_rate)
        .map(|a| a.to_btc() * BTC_PER_KB_TO_SAT_PER_VB)
        .unwrap_or(0.0)
}

/// Exponential backoff: base × 2^failures, capped at max.
fn backoff_delay(base: u64, max: u64, failures: u32) -> u64 {
    std::cmp::min(base * 2u64.pow(failures.min(4)), max)
}
