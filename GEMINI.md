# GEMINI Context: cyberdeckstr

This project is **cyberdeckstr**, a high-fidelity, cyberpunk-aesthetic Nostr client for the terminal (TUI).

## Project Overview
*   **Purpose:** A retro-future Nostr client designed for reading feeds and visualizing Bitcoin network data.
*   **Main Technologies:**
    *   **Language:** Rust (Edition 2024)
    *   **TUI Framework:** `ratatui` with `crossterm` backend.
    *   **Nostr Protocol:** `nostr-sdk` for relay connections and event handling.
    *   **Async Runtime:** `tokio` for concurrent networking and TUI polling.
    *   **Data Sources:** CoinGecko API (BTC price), optional Bitcoin Core RPC (blockchain visualization), and Linux sysfs/procfs (CPU, GPU, RAM, VRAM, and Network speed).
    *   **Logging:** `tracing` with `tracing-appender` for rolling file-based logs (writes to `logs/`).
*   **Architecture:**
    *   `src/main.rs`: Entry point, TUI initialization, panic guard, and the main event loop. Owns `App` directly (no shared-state locks).
    *   `src/app.rs`: Application state with domain sub-states (`FeedState`, `MarketState`, `NodeState`), the `AppMessage` enum for inter-task communication, and dirty-flag rendering.
    *   `src/ui.rs`: TUI rendering logic with named `LayoutSlots`, section renderers, and the cyberpunk theme.
    *   `src/nostr.rs`: Nostr connection lifecycle — key parsing, relay bootstrapping, follow-graph discovery, relay optimization, metadata resolution, subscription, and the notification event loop with health checks.
    *   `src/bitcoin.rs`: Bitcoin Core RPC polling with exponential backoff. Runs blocking calls via `spawn_blocking`.
    *   `src/market.rs`: CoinGecko HTTP polling for 24h BTC/USD price data.
    *   `src/system_stats.rs`: Background polling loop for CPU, RAM, AMD GPU, VRAM, and Network speed on Linux.
    *   `src/config.rs`: Configuration management via TOML.

## Building and Running
*   **Run:** `cargo run`
*   **Build Release:** `cargo build --release`
*   **Configuration:** Copy `config.toml.example` to `config.toml` and provide an `npub`. Bitcoin node settings are optional.
*   **Logs:** Rolling daily log files are written to `logs/cyberdeckstr.log`.

## Development Conventions
*   **Message-Passing Concurrency:** Background tasks communicate with the main loop via `mpsc::Sender<AppMessage>`. The main loop is the sole owner of `App` — no `Arc<Mutex>` contention. All state mutations flow through `App::handle_message()`.
*   **Domain Sub-States:** `App` delegates to `FeedState` (messages, dedup, text-wrap cache), `MarketState` (price history, cached chart bounds), and `NodeState` (blocks, mempool, fees). Each sub-state owns its data and methods.
*   **UI State:** Uses a "dirty flag" pattern in `App` to avoid redundant TUI renders. Named `LayoutSlots` replace index arithmetic for layout regions.
*   **Styling:** Cyberpunk aesthetic (Neon Green, Pink, Cyan) is strictly enforced in `src/ui.rs`. Colour constants are defined at module scope.
*   **Error Handling:** Uses `anyhow::Result` for application-level errors. `tracing` logs errors to file instead of silently discarding them. A panic hook restores terminal state before printing backtraces.
*   **Memory Management:** Bounded ring buffers for messages (`MAX_MESSAGES = 2000`) and seen event IDs (`MAX_SEEN_IDS = 5000`) with LRU eviction. Uses `EventId` (fixed-size) instead of heap-allocated `String` for dedup keys.
*   **Performance Caching:** Chart axis bounds are recomputed only when price data changes. Text wrapping is cached per-message and invalidated on terminal resize.

## Key Features to Maintain
*   **Identity Resolution:** Automatically resolves `npub` to display names via metadata (Kind 0) events.
*   **Relay Optimization:** Dynamically discovers and connects to relays based on the user's follow list (Kind 10002).
*   **Blockchain Viz:** Real-time dashboard for Bitcoin blocks, mempool, and fees when a node is configured.
*   **Adaptive Polling:** The main loop adjusts polling frequency based on user activity to save CPU.
*   **Connection Resilience:** Health checks detect stale connections (e.g. after system suspend) and auto-reconnect. Bitcoin RPC uses exponential backoff on failure.
*   **Terminal Safety:** Panic hook ensures the terminal is restored to normal mode even on crashes.
*   **System Monitor:** Real-time system monitoring (CPU, GPU, RAM, VRAM, Network speed) drawn cleanly inside the Status panel with zero external library overhead.
