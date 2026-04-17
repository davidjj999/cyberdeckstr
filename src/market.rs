use crate::app::AppMessage;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;

/// CoinGecko API response DTO (kept here, not in app.rs).
#[derive(Deserialize)]
pub struct MarketChart {
    pub prices: Vec<(f64, f64)>,
}

/// Long-running task that polls CoinGecko for 24h BTC/USD price data.
pub async fn fetch_btc_data(tx: mpsc::Sender<AppMessage>) {
    // Build a reusable HTTP client with connection pooling
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client — TLS or system error");

    loop {
        let url =
            "https://api.coingecko.com/api/v3/coins/bitcoin/market_chart?vs_currency=usd&days=1";

        match client
            .get(url)
            .header("User-Agent", "cyberdeckstr/1.0")
            .send()
            .await
        {
            Ok(resp) => match resp.json::<MarketChart>().await {
                Ok(chart_data) => {
                    let _ = tx.send(AppMessage::BtcPriceUpdate(chart_data.prices)).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse BTC market data: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to fetch BTC market data: {}", e);
            }
        }

        // CoinGecko free tier: poll every 60 seconds
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
