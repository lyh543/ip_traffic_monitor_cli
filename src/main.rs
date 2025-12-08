use chrono::Local;
use clap::Parser;
use diesel::prelude::*;
use hex::decode;
use procfs::process::Process;
use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::thread;
use std::time::Duration;
use std::str::FromStr;

// 导入 Diesel 生成的 schema（需按前版步骤生成）
mod schema;
use schema::ip_traffic;

// ==================== 命令行参数定义 ====================
#[derive(Parser, Debug)]
#[command(author, version, about = "基于 iftop 的精确 IP 流量统计工具", long_about = None)]
struct Cli {
    /// 出口网卡名（必填，通过 ip addr 查看）
    #[arg(short, long, required = true, help = "示例：eth0、ens33、enp2s0")]
    iface: String,

    /// 监控时长（单位：秒，默认 30 秒，设置为 0 表示永久运行）
    #[arg(short, long, default_value_t = 30, help = "示例：60（监控 1 分钟），0（永久运行）")]
    duration: u32,

    /// 数据库文件路径（默认：ip_traffic_stats_orm.db）
    #[arg(
        short = 'f',
        long,
        default_value = "ip_traffic_stats_orm.db",
        help = "示例：./data/traffic.db"
    )]
    db_path: String,

    /// iftop 采样间隔（默认 2 秒）
    #[arg(short = 's', long, default_value_t = 2, help = "iftop 采样间隔")]
    sample_interval: u32,
}

// ==================== ORM 模型 ====================
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = ip_traffic)]
struct NewIpTraffic {
    timestamp: String,
    remote_ip: String,
    tx_bytes: i32,
    pid: Option<i32>,
}

// ==================== 工具函数（复用前版逻辑） ====================
fn hex_to_ipv4(hex_str: &str) -> Option<Ipv4Addr> {
    let bytes = decode(hex_str).ok()?;
    if bytes.len() == 4 {
        Some(Ipv4Addr::new(bytes[3], bytes[2], bytes[1], bytes[0]))
    } else {
        None
    }
}

// ==================== 根据连接信息查找PID ====================
fn find_pid_for_connection(remote_ip: &str) -> Option<i32> {
    // 读取 /proc/net/tcp 查找对应的连接
    if let Ok(content) = std::fs::read_to_string("/proc/net/tcp") {
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 10 {
                // 解析远程地址
                if let Some(remote_addr_parts) = parts[2].split(':').collect::<Vec<_>>().get(0) {
                    if let Some(parsed_ip) = hex_to_ipv4(remote_addr_parts) {
                        if parsed_ip.to_string() == remote_ip {
                            if let Ok(inode) = parts[9].parse::<u32>() {
                                return find_pid_by_inode(inode).map(|p| p as i32);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_pid_by_inode(inode: u32) -> Option<u32> {
    std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            Process::new(pid as i32).ok()?.fd().ok()?.flatten().find_map(|fd| {
                match &fd.target {
                    procfs::process::FDTarget::Socket(socket_inode) => {
                        if *socket_inode == inode as u64 {
                            Some(pid)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
        })
}

// ==================== iftop 输出解析结构 ====================
#[derive(Debug, Clone)]
struct IftopConnection {
    local_ip: String,
    remote_ip: String,
    tx_bytes: f64,  // 发送字节数
    rx_bytes: f64,  // 接收字节数
}

// ==================== 格式化速率显示函数 ====================
fn format_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB/s", bytes_per_sec / (1024.0 * 1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.2} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.2} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

// ==================== 格式化字节数显示函数 ====================
fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.2} MB", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.2} KB", bytes / 1024.0)
    } else {
        format!("{:.0} B", bytes)
    }
}

// ==================== 单位转换函数 ====================
fn parse_rate_to_bytes_per_sec(rate_str: &str) -> Option<f64> {
    let rate_str = rate_str.trim();
    if rate_str.is_empty() || rate_str == "0" {
        return Some(0.0);
    }

    let (number_part, unit) = if rate_str.ends_with("Kb") {
        // Kb = Kilobits, 需要除以8转换为字节
        (rate_str.trim_end_matches("Kb"), 1024.0 / 8.0)
    } else if rate_str.ends_with("Mb") {
        // Mb = Megabits, 需要除以8转换为字节
        (rate_str.trim_end_matches("Mb"), 1024.0 * 1024.0 / 8.0)
    } else if rate_str.ends_with("Gb") {
        // Gb = Gigabits, 需要除以8转换为字节
        (rate_str.trim_end_matches("Gb"), 1024.0 * 1024.0 * 1024.0 / 8.0)
    } else if rate_str.ends_with("b") {
        // b = bits, 需要除以8转换为字节
        (rate_str.trim_end_matches("b"), 1.0 / 8.0)
    } else if rate_str.ends_with("B") {
        // B = Bytes, 直接使用
        (rate_str.trim_end_matches("B"), 1.0)
    } else {
        // 假设没有单位的是比特
        (rate_str, 1.0 / 8.0)
    };

    number_part.parse::<f64>().ok().map(|n| n * unit)
}

// ==================== iftop 输出解析函数 ====================
fn parse_iftop_output(output: &str, local_ip: &str, sample_interval: u32) -> Vec<IftopConnection> {
    let mut connections = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;
    
    while i < lines.len() {
        let line = lines[i].trim();
        
        // 查找发送行，格式：数字 本地IP => 速率1 速率2 速率3 累计
        if line.contains("=>") && line.contains(local_ip) {
            // 解析发送行获取发送速率
            let parts: Vec<&str> = line.split("=>").collect();
            if parts.len() == 2 {
                let right_part = parts[1].trim();
                let rate_tokens: Vec<&str> = right_part.split_whitespace().collect();
                
                if rate_tokens.len() >= 4 {
                    let tx_rate_str = rate_tokens[0]; // last 2s 速率
                    
                    if let Some(tx_rate) = parse_rate_to_bytes_per_sec(tx_rate_str) {
                        // 查找下一行的远程IP和接收速率
                        if i + 1 < lines.len() {
                            let next_line = lines[i + 1].trim();
                            if next_line.contains("<=") {
                                let rx_parts: Vec<&str> = next_line.split("<=").collect();
                                if rx_parts.len() == 2 {
                                    let left_part = rx_parts[0].trim();
                                    let right_part = rx_parts[1].trim();
                                    
                                    // 从左侧获取远程IP
                                    let ip_tokens: Vec<&str> = left_part.split_whitespace().collect();
                                    if let Some(&remote_ip) = ip_tokens.last() {
                                        // 验证是否为有效IP
                                        if let Ok(_) = Ipv4Addr::from_str(remote_ip) {
                                            // 从右侧获取接收速率
                                            let rx_rate_tokens: Vec<&str> = right_part.split_whitespace().collect();
                                            let rx_rate = if rx_rate_tokens.len() >= 1 {
                                                parse_rate_to_bytes_per_sec(rx_rate_tokens[0]).unwrap_or(0.0)
                                            } else {
                                                0.0
                                            };
                                            
                                            connections.push(IftopConnection {
                                                local_ip: local_ip.to_string(),
                                                remote_ip: remote_ip.to_string(),
                                                tx_bytes: tx_rate * sample_interval as f64,
                                                rx_bytes: rx_rate * sample_interval as f64,
                                            });
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
    
    connections
}

// ==================== 获取本地IP地址 ====================
fn get_local_ip(interface: &str) -> Result<String, String> {
    let output = Command::new("ip")
        .args(&["addr", "show", interface])
        .output()
        .map_err(|e| format!("执行 ip 命令失败: {}", e))?;
    
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
    Err(format!("无法获取网卡 {} 的IP地址", interface))
}

// ==================== 运行 iftop 并解析输出 ====================
fn run_iftop_and_parse(interface: &str, sample_interval: u32) -> Result<Vec<IftopConnection>, String> {
    let local_ip = get_local_ip(interface)?;
    
    let mut child = Command::new("sudo")
        .args(&[
            "iftop",
            "-i", interface,
            "-t",
            "-s", &sample_interval.to_string(),
            "-n",
            "-N"
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 iftop 失败: {}. 请确保已安装 iftop 并有 sudo 权限", e))?;

    // 读取输出
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

    // 等待进程结束
    let _ = child.wait();

    let connections = parse_iftop_output(&output, &local_ip, sample_interval);
    
    Ok(connections)
}

// ==================== 执行单次监控周期 ====================
fn run_monitor_cycle(iface: &str, sample_interval: u32, conn: &mut SqliteConnection, cycle_info: &str) -> Result<(), String> {
    println!("[{}] 正在采集流量数据...", cycle_info);
    
    match run_iftop_and_parse(iface, sample_interval) {
        Ok(connections) => {
            process_connections(&connections, conn)?;
        }
        Err(e) => {
            eprintln!("iftop 执行失败: {}", e);
            thread::sleep(Duration::from_secs(sample_interval as u64));
        }
    }
    
    Ok(())
}

// ==================== 主函数 ====================
fn main() -> Result<(), String> {
    let cli = Cli::parse();
    
    let is_permanent = cli.duration == 0;
    
    println!("基于 iftop 的精确IP流量监控工具");
    if is_permanent {
        println!("网卡: {}, 监控模式: 永久运行, 采样间隔: {}秒", 
                 cli.iface, cli.sample_interval);
        println!("提示: 按 Ctrl+C 停止监控");
    } else {
        println!("网卡: {}, 监控时长: {}秒, 采样间隔: {}秒", 
                 cli.iface, cli.duration, cli.sample_interval);
    }
    println!("数据库: {}", cli.db_path);
    println!("========================================");

    // 初始化数据库
    std::env::set_var("DATABASE_URL", &cli.db_path);
    let mut conn = SqliteConnection::establish(&cli.db_path)
        .map_err(|e| format!("数据库连接失败: {}", e))?;

    if is_permanent {
        // 永久运行模式
        let mut cycle = 1;
        loop {
            run_monitor_cycle(&cli.iface, cli.sample_interval, &mut conn, &format!("周期 {}", cycle))?;
            cycle += 1;
        }
    } else {
        // 定时运行模式
        let cycles = cli.duration / cli.sample_interval;
        
        for cycle in 1..=cycles {
            run_monitor_cycle(&cli.iface, cli.sample_interval, &mut conn, &format!("{}/{}", cycle, cycles))?;
        }
        
        println!("监控完成，数据已保存到 {}", cli.db_path);
    }
    
    Ok(())
}

// ==================== 处理连接数据的辅助函数 ====================
fn process_connections(connections: &[IftopConnection], conn: &mut SqliteConnection) -> Result<(), String> {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    
    if !connections.is_empty() {
        println!("[{}] 流量统计：", Local::now().format("%H:%M:%S"));
        
        for connection in connections {
            if connection.tx_bytes > 0.0 {
                let pid = find_pid_for_connection(&connection.remote_ip);
                
                let traffic = NewIpTraffic {
                    timestamp: timestamp.clone(),
                    remote_ip: connection.remote_ip.clone(),
                    tx_bytes: connection.tx_bytes as i32,
                    pid,
                };
                
                // 插入数据库
                if let Err(e) = diesel::insert_into(ip_traffic::table)
                    .values(&traffic)
                    .execute(conn) {
                    eprintln!("插入数据库失败: {}", e);
                }
                
                println!("  IP: {} | 出口字节: {} | 入口字节: {} | PID: {}",
                       connection.remote_ip,
                       format_bytes(connection.tx_bytes),
                       format_bytes(connection.rx_bytes),
                       pid.unwrap_or(0));
            }
        }
    } else {
        println!("[{}] 无活跃网络连接", Local::now().format("%H:%M:%S"));
    }
    
    Ok(())
}
