use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, List, ListItem, Paragraph},
    symbols,
    Frame,
};
use crate::app::{App, AppState, FeedEntryKind};

// ---------------------------------------------------------------------------
// Cyberpunk colour palette
// ---------------------------------------------------------------------------

const CYBER_GREEN: Color = Color::Rgb(0, 255, 65);
const CYBER_PINK: Color = Color::Rgb(255, 0, 255);
const CYBER_CYAN: Color = Color::Rgb(0, 240, 255);
const CYBER_BLACK: Color = Color::Rgb(10, 10, 16);

// Additional semantic colours for rich feed rendering
const REPOST_COLOR: Color = Color::Rgb(180, 130, 255);  // muted purple
const URL_COLOR: Color = Color::Rgb(80, 200, 255);      // bright blue

// ---------------------------------------------------------------------------
// Named layout slots — eliminates fragile index arithmetic
// ---------------------------------------------------------------------------

struct LayoutSlots {
    header: Rect,
    chart: Rect,
    blockchain_viz: Option<Rect>,
    content: Rect,
    status: Rect,
    sys_stats: Rect,
}

fn compute_layout(area: Rect, has_node: bool) -> LayoutSlots {
    let constraints = if has_node {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(10), // BTC Chart (smaller when node panel present)
            Constraint::Length(14), // Blockchain Viz
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Status Box (with stats inside)
            Constraint::Length(1), // Status Text (at the bottom)
        ]
    } else {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(12), // BTC Chart
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Status Box (with stats inside)
            Constraint::Length(1), // Status Text (at the bottom)
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(area);

    if has_node {
        LayoutSlots {
            header: chunks[0],
            chart: chunks[1],
            blockchain_viz: Some(chunks[2]),
            content: chunks[3],
            status: chunks[4],
            sys_stats: chunks[5],
        }
    } else {
        LayoutSlots {
            header: chunks[0],
            chart: chunks[1],
            blockchain_viz: None,
            content: chunks[2],
            status: chunks[3],
            sys_stats: chunks[4],
        }
    }
}

// ---------------------------------------------------------------------------
// Main render function
// ---------------------------------------------------------------------------

pub fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Styles
    let border_style = Style::default().fg(CYBER_PINK);
    let text_style = Style::default().fg(CYBER_GREEN).bg(CYBER_BLACK);
    let highlight_style = Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD);

    // Fill background
    f.render_widget(Block::default().style(Style::default().bg(CYBER_BLACK)), size);

    let slots = compute_layout(size, app.node.is_some());

    // -- Header --
    render_header(f, &slots, app, border_style);

    // -- BTC Chart --
    render_chart(f, slots.chart, app, border_style);

    // -- Blockchain Viz (optional) --
    if let Some(viz_area) = slots.blockchain_viz {
        render_blockchain_viz(f, viz_area, app, border_style, text_style);
    }

    // -- Content --
    render_content(f, slots.content, app, border_style, text_style, highlight_style);

    // -- Status Box (containing System Monitor Stats) --
    let stats_str = if let Some(stats) = &app.system_stats {
        format!(
            "CPU {:>3}%  GPU {:>3}%  RAM {:>3}%  VRAM {:>3}%  Network {:>10}",
            stats.cpu, stats.gpu, stats.ram, stats.vram, stats.network
        )
    } else {
        "CPU  --%  GPU  --%  RAM  --%  VRAM  --%  Network         --".to_string()
    };

    let status_box = Paragraph::new(stats_str)
        .style(Style::default().fg(Color::White).bg(CYBER_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" SYSTEM STATUS "),
        );
    f.render_widget(status_box, slots.status);

    // -- Status Message (at the very bottom) --
    let status_line = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(CYBER_PINK).bg(CYBER_BLACK));
    f.render_widget(status_line, slots.sys_stats);
}

// ---------------------------------------------------------------------------
// Section renderers
// ---------------------------------------------------------------------------

fn render_header(f: &mut Frame, slots: &LayoutSlots, app: &App, border_style: Style) {
    let header_text = match app.state {
        AppState::Login => "AUTH SEQUENCE",
        AppState::Connecting => "ESTABLISHING UPLINK",
        AppState::Feed => "DATA STREAM",
    };

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" CYBERDECKSTR 1.0 "),
        );
    f.render_widget(header, slots.header);
}

fn render_chart(f: &mut Frame, area: Rect, app: &App, border_style: Style) {
    let chart_block = Block::default()
        .title(" BTC/USD (24H) ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let datasets = vec![
        Dataset::default()
            .name("Price")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(CYBER_GREEN))
            .graph_type(GraphType::Line)
            .data(&app.market.btc_history),
    ];

    // Use pre-computed bounds from MarketState (no per-frame recalculation)
    let bounds = &app.market.chart_bounds;

    let chart = Chart::new(datasets)
        .block(chart_block)
        .x_axis(
            Axis::default()
                .title("Time")
                .style(Style::default().fg(Color::Gray))
                .bounds([bounds.min_x, bounds.max_x])
                .labels(vec![
                    Span::styled(" -24h ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(" Now ", Style::default().add_modifier(Modifier::BOLD)),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("Price")
                .style(Style::default().fg(Color::Gray))
                .bounds([bounds.min_y, bounds.max_y])
                .labels(vec![
                    Span::styled(
                        format!("{:.0}", bounds.min_y),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:.0}", bounds.max_y),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
        );
    f.render_widget(chart, area);
}

/// Split a text line into `Span`s, highlighting URLs in a distinct style.
fn style_line_with_urls(line: &str, base: Style, url_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, word) in line.split(' ').enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", base));
        }
        let is_url = word.starts_with("http://") || word.starts_with("https://");
        spans.push(Span::styled(word.to_string(), if is_url { url_style } else { base }));
    }
    if spans.is_empty() {
        spans.push(Span::styled(line.to_string(), base));
    }
    spans
}

fn render_blockchain_viz(
    f: &mut Frame,
    viz_area: Rect,
    app: &App,
    border_style: Style,
    text_style: Style,
) {
    let node = match &app.node {
        Some(n) => n,
        None => return,
    };

    let vz_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" BITCOIN MAINNET - {} ", node.status));
    f.render_widget(vz_block, viz_area);

    let inner_area = viz_area.inner(Margin { vertical: 1, horizontal: 1 });
    let viz_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(0)])
        .split(inner_area);

    // Block grid (6 columns)
    let block_constraints = vec![Constraint::Ratio(1, 6); 6];
    let block_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(block_constraints)
        .split(viz_rows[0]);

    // Cache current time once per frame
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    for (i, block) in node.blocks.iter().enumerate() {
        if i >= 6 {
            break;
        }

        let diff = now.saturating_sub(block.timestamp);
        let time_str = if diff < 60 {
            format!("{}s", diff)
        } else {
            format!("{}m", diff / 60)
        };

        let b_text = format!(
            "{}\n{} txs\n{}\n{:.2}MB",
            block.height,
            block.tx_count,
            time_str,
            block.size as f64 / 1_000_000.0
        );

        let b_widget = Paragraph::new(b_text)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(CYBER_GREEN))
                    .title("BLK"),
            );
        f.render_widget(b_widget, block_cols[i]);
    }

    // Bottom row: Fees + Mempool
    let stat_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(viz_rows[1]);

    // Fees
    let fee_text = format!(
        "LOW: {:.0} sat/vB\nMED: {:.0} sat/vB\nHIGH: {:.0} sat/vB",
        node.fees.low, node.fees.medium, node.fees.high
    );
    let fees_widget = Paragraph::new(fee_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(CYBER_PINK))
                .title(" FEES "),
        )
        .style(text_style);
    f.render_widget(fees_widget, stat_cols[0]);

    // Mempool gauge
    let mem_mb = node.mempool.usage as f64 / 1_000_000.0;
    let max_mb = node.mempool.max_mempool as f64 / 1_000_000.0;
    let ratio = (mem_mb / max_mb).clamp(0.0, 1.0);

    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" MEMPOOL RAM "))
        .gauge_style(Style::default().fg(CYBER_CYAN))
        .ratio(ratio)
        .label(format!(
            "{:.1} / {:.1} MB ({} txs)",
            mem_mb, max_mb, node.mempool.size
        ));
    f.render_widget(mem_gauge, stat_cols[1]);
}

fn render_content(
    f: &mut Frame,
    area: Rect,
    app: &App,
    border_style: Style,
    text_style: Style,
    highlight_style: Style,
) {
    match app.state {
        AppState::Login => {
            let input = Paragraph::new(app.input.as_str())
                .style(text_style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(" ENTER NPUB IDENTITY "),
                );
            f.render_widget(input, area);
        }
        AppState::Connecting => {
            let loading = Paragraph::new("DECRYPTING REALITY...")
                .style(
                    Style::default()
                        .fg(CYBER_GREEN)
                        .add_modifier(Modifier::RAPID_BLINK),
                )
                .block(Block::default().borders(Borders::ALL).border_style(border_style));
            f.render_widget(loading, area);
        }
        AppState::Feed => {
            let url_style = Style::default().fg(URL_COLOR).add_modifier(Modifier::UNDERLINED);

            // Build rich ListItems from entries + cached wrapped content
            let messages: Vec<ListItem> = app
                .feed
                .entries
                .iter()
                .zip(app.feed.wrapped_content.iter())
                .map(|(entry, wrapped_lines)| {
                    let mut lines: Vec<Line> = Vec::new();

                    // ── Header line ──────────────────────────────────
                    let mut header_spans: Vec<Span> = Vec::new();

                    // Kind: Repost marker
                    if entry.kind == FeedEntryKind::Repost {
                        header_spans.push(Span::styled(
                            "↻ ",
                            Style::default().fg(REPOST_COLOR).add_modifier(Modifier::BOLD),
                        ));
                    }

                    // Author name + NIP-05 badge + reply indicator
                    // all use CYBER_GREEN so the header reads as one unit.
                    // (The selected-item cyan is handled by highlight_style on the List widget.)
                    let header_style = Style::default().fg(CYBER_GREEN).add_modifier(Modifier::BOLD);

                    header_spans.push(Span::styled(
                        format!("@{}", entry.author),
                        header_style,
                    ));

                    if let Some(nip05) = &entry.nip05 {
                        header_spans.push(Span::raw(" "));
                        header_spans.push(Span::styled(
                            format!("[✓{}]", nip05),
                            header_style,
                        ));
                    }

                    if entry.is_reply {
                        header_spans.push(Span::raw(" "));
                        header_spans.push(Span::styled("(reply)", header_style));
                    }

                    lines.push(Line::from(header_spans));

                    // ── Content lines with URL highlighting ─────────
                    for line_str in wrapped_lines {
                        let styled_spans = style_line_with_urls(line_str, text_style, url_style);
                        lines.push(Line::from(styled_spans));
                    }

                    // Spacer between entries
                    lines.push(Line::from(Span::raw("")));

                    ListItem::new(lines)
                })
                .collect();

            let messages_list = List::new(messages)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(" LIVE FEED ")
                        .style(Style::default().bg(CYBER_BLACK)),
                )
                .highlight_style(highlight_style);

            let mut state = ratatui::widgets::ListState::default();
            state.select(Some(app.scroll));

            f.render_stateful_widget(messages_list, area, &mut state);
        }
    }
}


