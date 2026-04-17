use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, Paragraph},
    symbols,
    Frame,
};
use crate::app::{App, AppState};

const CYBER_GREEN: Color = Color::Rgb(0, 255, 65);
const CYBER_PINK: Color = Color::Rgb(255, 0, 255);
const CYBER_CYAN: Color = Color::Rgb(0, 240, 255);
const CYBER_BLACK: Color = Color::Rgb(10, 10, 16); // Very dark, almost black

pub fn ui(f: &mut Frame, app: &App) {
    let size = f.area();
    
    // Cyberpunk Style
    let border_style = Style::default().fg(CYBER_PINK);
    let text_style = Style::default().fg(CYBER_GREEN).bg(CYBER_BLACK);
    let highlight_style = Style::default().fg(CYBER_CYAN).add_modifier(Modifier::BOLD);

    // 1. Fill background to unify color (Black/Grey issue)
    f.render_widget(Block::default().style(Style::default().bg(CYBER_BLACK)), size);

    let constraints = if app.bitcoin_node_configured {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(10), // BTC Chart (Small)
            Constraint::Length(14), // Viz
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Status
        ]
    } else {
        vec![
            Constraint::Length(3),  // Header
            Constraint::Length(12), // BTC Chart
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Status
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
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

    // Helper to render Chart
    let render_chart = |f: &mut Frame, area: ratatui::layout::Rect, app: &App| {
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
                .data(&app.btc_history),
        ];
    
        // Calculate bounds
         let (min_x, max_x, min_y, max_y) = if app.btc_history.is_empty() {
            (0.0, 100.0, 0.0, 100.0)
        } else {
            let xs: Vec<f64> = app.btc_history.iter().map(|(x, _)| *x).collect();
            let ys: Vec<f64> = app.btc_history.iter().map(|(_, y)| *y).collect();
            (
                xs.first().cloned().unwrap_or(0.0),
                xs.last().cloned().unwrap_or(100.0),
                ys.iter().cloned().fold(f64::INFINITY, f64::min),
                ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            )
        };
    
        let chart = Chart::new(datasets)
            .block(chart_block)
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_x, max_x])
                    .labels(vec![
                        Span::styled(" -24h ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(" Now ", Style::default().add_modifier(Modifier::BOLD)),
                    ]),
            )
            .y_axis(
                Axis::default()
                    .title("Price")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_y, max_y])
                    .labels(vec![
                        Span::styled(format!("{:.0}", min_y), Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:.0}", max_y), Style::default().add_modifier(Modifier::BOLD)),
                    ]),
            );
        f.render_widget(chart, area);
    };

    // Visualization Area & Chart
    if app.bitcoin_node_configured {
        // 1. BTC Chart
        render_chart(f, chunks[1], app);
        
        // 2. Blockchain Viz
        let viz_area = chunks[2];
        let vz_block = Block::default().borders(Borders::ALL).border_style(border_style).title(format!(" BITCOIN MAINNET - {} ", app.node_status));
        f.render_widget(vz_block, viz_area);
        
        let inner_area = viz_area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 1 });
        let viz_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // Blocks row
                Constraint::Min(0),    // Rest
            ].as_ref())
            .split(inner_area);
            
        // Render Blocks
        let block_constraints = vec![Constraint::Ratio(1, 6); 6];
        let block_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(block_constraints)
            .split(viz_rows[0]);
        
        // Get current time once per frame instead of per block
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            
        for (i, block) in app.blocks.iter().enumerate() {
             if i >= 6 { break; }
             
             // Time diff (using cached 'now')
             let diff = now.saturating_sub(block.timestamp);
             let time_str = if diff < 60 { format!("{}s", diff) } else { format!("{}m", diff / 60) };
             
             let b_text = format!(
                 "{}\n{} txs\n{}\n{:.2}MB",
                 block.height,
                 block.tx_count,
                 time_str,
                 block.size as f64 / 1_000_000.0
             );
             
             let b_widget = Paragraph::new(b_text)
                .alignment(ratatui::layout::Alignment::Center)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(CYBER_GREEN)).title("BLK"));
             f.render_widget(b_widget, block_cols[i]);
        }
        
        // Bottom Row: Fees + Mempool
        let stat_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ].as_ref())
            .split(viz_rows[1]);
            
        // Fees
        let fee_text = format!(
            "LOW: {:.0} sat/vB\nMED: {:.0} sat/vB\nHIGH: {:.0} sat/vB",
            app.fees.low, app.fees.medium, app.fees.high
        );
        let fees_widget = Paragraph::new(fee_text)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(CYBER_PINK)).title(" FEES "))
            .style(text_style);
        f.render_widget(fees_widget, stat_cols[0]);
        
        // Mempool
        let mem_mb = app.mempool.usage as f64 / 1_000_000.0;
        let max_mb = app.mempool.max_mempool as f64 / 1_000_000.0;
        let ratio = (mem_mb / max_mb).clamp(0.0, 1.0);
        
        let mem_gauge = ratatui::widgets::Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" MEMPOOL RAM "))
            .gauge_style(Style::default().fg(CYBER_CYAN))
            .ratio(ratio)
            .label(format!("{:.1} / {:.1} MB ({} txs)", mem_mb, max_mb, app.mempool.size));
        f.render_widget(mem_gauge, stat_cols[1]);

    } else {
        // BTC Chart (Legacy behavior)
        render_chart(f, chunks[1], app);
    }

    // Content
    let content_chunk_index = if app.bitcoin_node_configured { 3 } else { 2 };
    
    match app.state {
        AppState::Login => {
            let input = Paragraph::new(app.input.as_str())
                .style(text_style)
                .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" ENTER NPUB IDENTITY "));
            f.render_widget(input, chunks[content_chunk_index]);
        }
        AppState::Connecting => {
             let loading = Paragraph::new("DECRYPTING REALITY...")
                .style(Style::default().fg(CYBER_GREEN).add_modifier(Modifier::RAPID_BLINK))
                .block(Block::default().borders(Borders::ALL).border_style(border_style));
             f.render_widget(loading, chunks[content_chunk_index]);
        }
        AppState::Feed => {
            // Calculate available width for text
            let max_width = chunks[content_chunk_index].width.saturating_sub(6) as usize;

            let messages: Vec<ListItem> = app.messages
                .iter()
                .map(|m| {
                    let mut lines = vec![Line::from(Span::raw(""))]; // Spacer
                    
                    // Split on newlines first, then wrap each line to avoid \n in Spans
                    for line in m.lines() {
                        if line.is_empty() {
                            lines.push(Line::from(Span::styled("", text_style)));
                            continue;
                        }
                        
                        let wrapped_lines = textwrap::wrap(line, max_width);
                        for w in wrapped_lines {
                            lines.push(Line::from(Span::styled(w.to_string(), text_style)));
                        }
                    }
                    
                    ListItem::new(lines)
                })
                .collect();
                
            let messages_list = List::new(messages)
                .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" LIVE FEED ").style(Style::default().bg(CYBER_BLACK)))
                .highlight_style(highlight_style); 

            let mut state = ratatui::widgets::ListState::default();
            state.select(Some(app.scroll));
            
            f.render_stateful_widget(messages_list, chunks[content_chunk_index], &mut state);
        }
    }

    // Footer / Status
    let status_chunk_index = if app.bitcoin_node_configured { 4 } else { 3 };
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::White).bg(CYBER_BLACK))
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(" SYSTEM STATUS "));
    f.render_widget(status, chunks[status_chunk_index]);
}

