use crate::monitor::{TrafficMonitor, TrafficStats};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::str::FromStr;

/// 基于 iftop 的流量监控器
pub struct IftopMonitor {
    interface: String,
    sample_interval: u32,
    local_ip: Option<String>,
}

impl IftopMonitor {
    pub fn new(interface: String, sample_interval: u32) -> Self {
        Self {
            interface,
            sample_interval,
            local_ip: None,
        }
    }

    /// 获取本地IP地址
    fn get_local_ip(&self) -> Result<String, Box<dyn Error>> {
        let output = Command::new("ip")
            .args(&["addr", "show", &self.interface])
            .output()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        for line in output_str.lines() {
            if line.trim().starts_with("inet ") && !line.contains("127.0.0.1") {
                let parts: Vec<&str> = line.trim().split_whitespace().collect();
                if let Some(ip_with_mask) = parts.get(1) {
                    if let Some(ip) = ip_with_mask.split('/').next() {
                        return Ok(ip.to_string());
                    }
                }
            }
        }
        Err(format!("无法获取网卡 {} 的IP地址", self.interface).into())
    }

    /// 解析速率字符串为每秒字节数
    fn parse_rate_to_bytes_per_sec(rate_str: &str) -> Option<f64> {
        let rate_str = rate_str.trim();
        if rate_str.is_empty() || rate_str == "0" {
            return Some(0.0);
        }

        let (number_part, unit) = if rate_str.ends_with("Kb") {
            (rate_str.trim_end_matches("Kb"), 1024.0 / 8.0)
        } else if rate_str.ends_with("Mb") {
            (rate_str.trim_end_matches("Mb"), 1024.0 * 1024.0 / 8.0)
        } else if rate_str.ends_with("Gb") {
            (rate_str.trim_end_matches("Gb"), 1024.0 * 1024.0 * 1024.0 / 8.0)
        } else if rate_str.ends_with("b") {
            (rate_str.trim_end_matches("b"), 1.0 / 8.0)
        } else if rate_str.ends_with("B") {
            (rate_str.trim_end_matches("B"), 1.0)
        } else {
            (rate_str, 1.0 / 8.0)
        };

        number_part.parse::<f64>().ok().map(|n| n * unit)
    }

    /// 解析 iftop 输出
    fn parse_iftop_output(&self, output: &str) -> HashMap<String, TrafficStats> {
        let mut stats_map = HashMap::new();
        let local_ip = match &self.local_ip {
            Some(ip) => ip,
            None => return stats_map,
        };

        let lines: Vec<&str> = output.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i].trim();

            if line.contains("=>") && line.contains(local_ip) {
                let parts: Vec<&str> = line.split("=>").collect();
                if parts.len() == 2 {
                    let right_part = parts[1].trim();
                    let rate_tokens: Vec<&str> = right_part.split_whitespace().collect();

                    if rate_tokens.len() >= 4 {
                        let tx_rate_str = rate_tokens[0];

                        if let Some(tx_rate) = Self::parse_rate_to_bytes_per_sec(tx_rate_str) {
                            if i + 1 < lines.len() {
                                let next_line = lines[i + 1].trim();
                                if next_line.contains("<=") {
                                    let rx_parts: Vec<&str> = next_line.split("<=").collect();
                                    if rx_parts.len() == 2 {
                                        let left_part = rx_parts[0].trim();
                                        let right_part = rx_parts[1].trim();

                                        let ip_tokens: Vec<&str> = left_part.split_whitespace().collect();
                                        if let Some(&remote_ip) = ip_tokens.last() {
                                            if Ipv4Addr::from_str(remote_ip).is_ok() {
                                                let rx_rate_tokens: Vec<&str> =
                                                    right_part.split_whitespace().collect();
                                                let rx_rate = if !rx_rate_tokens.is_empty() {
                                                    Self::parse_rate_to_bytes_per_sec(rx_rate_tokens[0])
                                                        .unwrap_or(0.0)
                                                } else {
                                                    0.0
                                                };

                                                let tx_bytes =
                                                    (tx_rate * self.sample_interval as f64) as u64;
                                                let rx_bytes =
                                                    (rx_rate * self.sample_interval as f64) as u64;

                                                stats_map.insert(
                                                    remote_ip.to_string(),
                                                    TrafficStats {
                                                        tx_bytes,
                                                        rx_bytes,
                                                        tx_packets: 0, // iftop 不提供包数
                                                        rx_packets: 0,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            i += 1;
        }

        stats_map
    }
}

impl TrafficMonitor for IftopMonitor {
    fn init(&mut self) -> Result<(), Box<dyn Error>> {
        self.local_ip = Some(self.get_local_ip()?);
        println!("iftop 监控器初始化成功，本地IP: {}", self.local_ip.as_ref().unwrap());
        Ok(())
    }

    fn start(&mut self) -> Result<HashMap<String, TrafficStats>, Box<dyn Error>> {
        let mut child = Command::new("iftop")
            .args(&[
                "-i",
                &self.interface,
                "-t",
                "-s",
                &self.sample_interval.to_string(),
                "-n",
                "-N",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut output = String::new();
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
        }

        let _ = child.wait();

        Ok(self.parse_iftop_output(&output))
    }

    fn stop(&mut self) -> Result<(), Box<dyn Error>> {
        // iftop 是同步执行的，不需要额外停止操作
        Ok(())
    }

    fn name(&self) -> &str {
        "iftop"
    }
}


