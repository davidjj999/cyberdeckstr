use std::fs;
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use crate::app::AppMessage;

pub async fn monitor_system_stats(tx: mpsc::Sender<AppMessage>) {
    let amd_gpu = find_amd_gpu_paths();
    let mut prev_cpu = read_cpu_stats().ok();
    let mut prev_net_bytes = read_network_bytes().ok();
    let mut prev_net_time = Instant::now();

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;

        // CPU
        let mut cpu_pct = 0;
        if let Ok((curr_total, curr_idle)) = read_cpu_stats() {
            if let Some((prev_total, prev_idle)) = prev_cpu {
                let total_diff = curr_total.saturating_sub(prev_total);
                let idle_diff = curr_idle.saturating_sub(prev_idle);
                if total_diff > 0 {
                    cpu_pct = (100.0 * (total_diff - idle_diff) as f64 / total_diff as f64) as u8;
                }
            }
            prev_cpu = Some((curr_total, curr_idle));
        }

        // RAM
        let ram_pct = read_ram_stats().unwrap_or(0);

        // GPU / VRAM
        let (gpu_pct, vram_pct) = if let Some(ref paths) = amd_gpu {
            read_gpu_stats(paths)
        } else {
            (0, 0)
        };

        // Network
        let mut net_str = "0 B/s".to_string();
        if let Ok(curr_bytes) = read_network_bytes() {
            if let Some(prev_bytes) = prev_net_bytes {
                let elapsed = prev_net_time.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let bytes_diff = curr_bytes.saturating_sub(prev_bytes);
                    let speed = bytes_diff as f64 / elapsed;
                    net_str = format_network_speed(speed);
                }
            }
            prev_net_bytes = Some(curr_bytes);
            prev_net_time = Instant::now();
        }

        let msg = AppMessage::SystemStatsUpdate {
            cpu: cpu_pct.min(100),
            gpu: gpu_pct.min(100),
            ram: ram_pct.min(100),
            vram: vram_pct.min(100),
            network: net_str,
        };

        if tx.send(msg).await.is_err() {
            break;
        }
    }
}

fn read_cpu_stats() -> io::Result<(u64, u64)> {
    let content = fs::read_to_string("/proc/stat")?;
    for line in content.lines() {
        if line.starts_with("cpu ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let user: u64 = parts[1].parse().unwrap_or(0);
                let nice: u64 = parts[2].parse().unwrap_or(0);
                let system: u64 = parts[3].parse().unwrap_or(0);
                let idle: u64 = parts[4].parse().unwrap_or(0);
                let iowait: u64 = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
                let irq: u64 = parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
                let softirq: u64 = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
                let steal: u64 = parts.get(8).and_then(|s| s.parse().ok()).unwrap_or(0);

                let idle_time = idle + iowait;
                let total_time = user + nice + system + idle_time + irq + softirq + steal;
                return Ok((total_time, idle_time));
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "cpu line not found in /proc/stat"))
}

fn read_ram_stats() -> io::Result<u8> {
    let content = fs::read_to_string("/proc/meminfo")?;
    let mut mem_total = None;
    let mut mem_available = None;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            mem_total = line.split_whitespace().nth(1).and_then(|s| s.parse::<u64>().ok());
        } else if line.starts_with("MemAvailable:") {
            mem_available = line.split_whitespace().nth(1).and_then(|s| s.parse::<u64>().ok());
        }
        if mem_total.is_some() && mem_available.is_some() {
            break;
        }
    }
    if let (Some(total), Some(avail)) = (mem_total, mem_available) {
        if total > 0 {
            let used = total.saturating_sub(avail);
            let percentage = (used as f64 / total as f64 * 100.0) as u8;
            return Ok(percentage.min(100));
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "RAM stats not found"))
}

fn find_amd_gpu_paths() -> Option<(String, String, String)> {
    let mut best_card = None;
    let mut max_vram = 0;

    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("card") && !name.contains('-') {
                let dev_path = entry.path().join("device");
                let vram_total_path = dev_path.join("mem_info_vram_total");
                if vram_total_path.exists() {
                    if let Ok(vram_str) = fs::read_to_string(&vram_total_path) {
                        if let Ok(vram_val) = vram_str.trim().parse::<u64>() {
                            if vram_val > max_vram {
                                max_vram = vram_val;
                                best_card = Some(dev_path);
                            }
                        }
                    }
                }
            }
        }
    }

    best_card.map(|path| {
        (
            path.join("gpu_busy_percent").to_string_lossy().into_owned(),
            path.join("mem_info_vram_used").to_string_lossy().into_owned(),
            path.join("mem_info_vram_total").to_string_lossy().into_owned(),
        )
    })
}

fn read_gpu_stats(paths: &(String, String, String)) -> (u8, u8) {
    let gpu_busy = fs::read_to_string(&paths.0)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0);

    let vram_used = fs::read_to_string(&paths.1)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let vram_total = fs::read_to_string(&paths.2)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let vram_pct = if vram_total > 0 {
        (vram_used as f64 / vram_total as f64 * 100.0) as u8
    } else {
        0
    };

    (gpu_busy.min(100), vram_pct.min(100))
}

fn read_network_bytes() -> io::Result<u64> {
    let content = fs::read_to_string("/proc/net/dev")?;
    let mut total_bytes = 0;
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 10 {
            let iface = parts[0].trim_end_matches(':');
            if iface != "lo" && !iface.is_empty() {
                let rx_bytes: u64 = parts[1].parse().unwrap_or(0);
                let tx_bytes: u64 = parts[9].parse().unwrap_or(0);
                total_bytes += rx_bytes + tx_bytes;
            }
        }
    }
    Ok(total_bytes)
}

fn format_network_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 {
        format!("{:.0} B/s", bytes_per_sec)
    } else if bytes_per_sec < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    }
}
