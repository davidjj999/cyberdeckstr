# CYBERDECKSTR

A high-fidelity, cyberpunk-aesthetic Nostr client for your terminal.

![Cyberdeckstr Screenshot](screenshot.png)

## Description

CYBERDECKSTR is a TUI (Terminal User Interface) client for the Nostr protocol, built in Rust. It is designed for those who prefer the command line and enjoy a bit of retro-future flair.

**Features:**
*   **Read-Only Feed:** Connects securely to Nostr relays.
*   **Identity Resolution:** Automatically resolves `npub` identities to display names.
*   **Cyberpunk UI:** Custom neon styling using `ratatui`.
*   **Lightning Fast:** Built on `tokio` for asynchronous performance.

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
2.  Paste your **npub** (public key) when prompted.
3.  The client will jack into the matrix, fetch your follows, and display a live stream of their notes.

## License

MIT
