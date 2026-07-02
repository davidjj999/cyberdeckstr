# CYBERDECKSTR

A high-fidelity, cyberpunk-aesthetic Nostr client for your terminal.

![Cyberdeckstr Screenshot](screenshot.png)

## Description

CYBERDECKSTR is a TUI (Terminal User Interface) client for the Nostr protocol, built in Rust. It is designed for those who prefer the command line and enjoy a bit of retro-future flair.

**Features:**
*   **Rich Feed Format:** Reposts marked with `↻`, reply chains detected via `#e` tags, NIP-05 verified authors show a `[✓domain]` badge, and URLs are highlighted in distinct blue.
*   **Read-Only Feed:** Connects to Nostr relays and streams notes from your follow list.
*   **Identity Resolution:** Automatically resolves `npub` identities to display names and NIP-05 identifiers via Kind 0 metadata.
*   **Relay Optimization:** Discovers and connects to relays where your follows actually post (Kind 10002).
*   **Bitcoin Dashboard:** Optional real-time visualization of blocks, mempool, and fee estimates from a local Bitcoin Core node.
*   **BTC Price Chart:** 24-hour BTC/USD price chart via CoinGecko (always on).
*   **System Monitor:** Live CPU, GPU, RAM, VRAM, and network speed readouts — zero external dependencies.
*   **Cyberpunk UI:** Custom neon styling (green, pink, cyan on black) using `ratatui`.
*   **Lightning Fast:** Lock-free, message-passing architecture built on `tokio`.
*   **Connection Resilience:** Auto-reconnects on stale connections and system suspend/resume — subscriptions are re-issued so the feed never goes silent.

## Installation

### Prerequisites
*   [Rust and Cargo](https://rustup.rs/)

### Build from Source

1.  Clone the repository:
    ```bash
    git clone https://github.com/davidjj999/cyberdeckstr.git
    cd cyberdeckstr
    ```

2.  Run the application:
    ```bash
    cargo run
    ```

3.  Build for release:
    ```bash
    cargo build --release
    ./target/release/cyberdeckstr
    ```

## Usage

1.  Launch the application.
2.  Paste your **npub** (public key) when prompted (or configure it in `config.toml` for auto-login).
3.  The client will jack into the matrix, discover your follows' preferred relays, resolve their identities, and display a live stream of their notes.
4.  **Navigate:** `↑`/`↓` to scroll through the feed, `q` or `Esc` to quit.

    Entries show event metadata inline:
    *   `↻ @author` — a repost (Kind 6).
    *   `@author [✓user@domain]` — NIP-05 verified author.
    *   `@author (reply)` — a note that replies to another event.
    *   Blue underlined text — detected URLs in note content.

## Configuration (Optional)

Create a `config.toml` file in the project root to automate login and enable the Bitcoin dashboard.

1.  Copy the example config:
    ```bash
    cp config.toml.example config.toml
    ```
2.  Edit `config.toml` with your details:
    ```toml
    # Auto-login with your npub
    npub = "npub1..."

    # Bitcoin Node Connection (Optional)
    # Leave blank or remove to disable the blockchain dashboard
    node_address = "127.0.0.1:8332"
    node_username = "your_rpc_user"
    node_password = "your_rpc_password"
    ```

**What configuration enables:**
- **Auto-Login:** Skips the manual npub entry screen.
- **Blockchain Dashboard:** Adds a real-time panel showing the latest 6 blocks, fee estimates (low/med/high), and mempool usage gauge. Requires a running Bitcoin Core node with RPC enabled.

## Logging

CYBERDECKSTR writes rolling daily log files to `logs/cyberdeckstr.log`. These capture connection events, relay discovery, errors, and health-check activity — useful for diagnosing issues in long-running sessions without interfering with the TUI.

## Architecture

```
main.rs          — Event loop (owns App, no locks)
  ├─ nostr.rs    — Nostr connection lifecycle & event streaming
  ├─ bitcoin.rs  — Bitcoin Core RPC polling (spawn_blocking)
  ├─ market.rs   — CoinGecko price data polling
  ├─ app.rs      — App state, AppMessage enum, sub-states
  ├─ ui.rs       — Rendering (LayoutSlots, section renderers)
  └─ config.rs   — TOML config parsing
```

Background tasks send `AppMessage` variants through an `mpsc` channel to the main loop, which is the sole owner of application state. No shared-state locks.

## License

MIT
