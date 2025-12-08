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
use actix_web::{web, App, HttpServer, HttpResponse};
use maxminddb::{geoip2, Reader};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

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

    /// 启用 Prometheus exporter（默认端口：9090）
    #[arg(short = 'p', long, help = "启用 Prometheus exporter 监听端口")]
    prometheus_port: Option<u16>,

    /// GeoIP2 数据库文件路径（可选，用于 IP 地理位置查询）
    #[arg(short = 'g', long, help = "GeoIP2 City 数据库文件路径，例如：GeoLite2-City.mmdb")]
    geoip_db: Option<String>,

    /// Prometheus metrics 流量阈值（单位：字节，默认 1MB）
    #[arg(short = 't', long, default_value_t = 1024 * 1024, help = "低于此阈值的流量不会导出到 Prometheus")]
    prometheus_export_threshold: i64,
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

#[derive(Debug, Clone, Queryable)]
#[diesel(table_name = ip_traffic)]
struct IpTrafficRecord {
    id: i32,
    timestamp: String,
    remote_ip: String,
    tx_bytes: i32,
    pid: Option<i32>,
}

// ==================== Prometheus Exporter 相关 ====================
// 全局 GeoIP 数据库读取器
static GEOIP_READER: Lazy<Mutex<Option<Reader<Vec<u8>>>>> = Lazy::new(|| Mutex::new(None));

// 全局退出标志
static RUNNING: AtomicBool = AtomicBool::new(true);

// IP 地理信息结构
#[derive(Debug, Clone)]
struct IpGeoInfo {
    country: String,
    province: String,
    city: String,
    isp: String,
}

fn init_geoip_db(db_path: &str) -> Result<(), String> {
    match Reader::open_readfile(db_path) {
        Ok(reader) => {
            *GEOIP_READER.lock().unwrap() = Some(reader);
            println!("GeoIP 数据库加载成功: {}", db_path);
            Ok(())
        }
        Err(e) => Err(format!("GeoIP 数据库加载失败: {}", e)),
    }
}

fn get_ip_geo_info(ip_str: &str) -> IpGeoInfo {
    let default_info = IpGeoInfo {
        country: "Unknown".to_string(),
        province: "Unknown".to_string(),
        city: "Unknown".to_string(),
        isp: "Unknown".to_string(),
    };

    // 如果没有加载 GeoIP 数据库，返回默认值
    let reader_guard = GEOIP_READER.lock().unwrap();
    let reader = match reader_guard.as_ref() {
        Some(r) => r,
        None => return default_info,
    };

    // 解析 IP 地址
    let ip: std::net::IpAddr = match ip_str.parse() {
        Ok(ip) => ip,
        Err(_) => return default_info,
    };

    // 查询 GeoIP 数据库
    match reader.lookup::<geoip2::City>(ip) {
        Ok(city) => {
            let country = if let Some(c) = &city.country {
                if let Some(names) = &c.names {
                    names.get("zh-CN")
                        .or_else(|| names.get("en"))
                        .unwrap_or(&"Unknown")
                        .to_string()
                } else {
                    "Unknown".to_string()
                }
            } else {
                "Unknown".to_string()
            };

            let province = if let Some(subdivisions) = &city.subdivisions {
                if let Some(first) = subdivisions.first() {
                    if let Some(names) = &first.names {
                        names.get("zh-CN")
                            .or_else(|| names.get("en"))
                            .unwrap_or(&"Unknown")
                            .to_string()
                    } else {
                        "Unknown".to_string()
                    }
                } else {
                    "Unknown".to_string()
                }
            } else {
                "Unknown".to_string()
            };

            let city_name = if let Some(c) = &city.city {
                if let Some(names) = &c.names {
                    names.get("zh-CN")
                        .or_else(|| names.get("en"))
                        .unwrap_or(&"Unknown")
                        .to_string()
                } else {
                    "Unknown".to_string()
                }
            } else {
                "Unknown".to_string()
            };

            // GeoLite2-City 数据库不包含 ISP 详细信息
            // 如需 ISP 信息，建议使用纯真 IP 数据库或付费的 GeoIP2-ISP 数据库
            let isp = "Unknown".to_string();

            IpGeoInfo {
                country,
                province,
                city: city_name,
                isp,
            }
        }
        Err(_) => default_info,
    }
}

#[derive(Clone)]
struct AppState {
    db_path: String,
    prometheus_export_threshold: i64,
}

async fn metrics_handler(data: web::Data<AppState>) -> HttpResponse {
    let db_path = &data.db_path;
    let prometheus_export_threshold = data.prometheus_export_threshold;
    
    match get_ip_traffic_metrics(db_path, prometheus_export_threshold) {
        Ok(metrics) => HttpResponse::Ok()
            .content_type("text/plain; version=0.0.4")
            .body(metrics),
        Err(e) => HttpResponse::InternalServerError()
            .body(format!("Error generating metrics: {}", e)),
    }
}

fn get_ip_traffic_metrics(db_path: &str, prometheus_export_threshold: i64) -> Result<String, String> {
    let mut conn = SqliteConnection::establish(db_path)
        .map_err(|e| format!("数据库连接失败: {}", e))?;
    
    use schema::ip_traffic::dsl::*;
    use diesel::dsl::sum;
    
    // 查询每个 IP 的累计流量
    let results: Vec<(String, Option<i64>)> = ip_traffic
        .group_by(remote_ip)
        .select((remote_ip, sum(tx_bytes)))
        .load(&mut conn)
        .map_err(|e| format!("查询数据库失败: {}", e))?;
    
    // 生成 Prometheus 格式的输出
    let mut output = String::new();
    output.push_str("# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address\n");
    output.push_str("# TYPE ip_traffic_tx_bytes_total counter\n");
    
    for (ip, total_bytes) in results {
        if let Some(bytes) = total_bytes {
            if bytes <= prometheus_export_threshold {
                continue;
            }
            // 获取 IP 地理信息
            let geo_info = get_ip_geo_info(&ip);
            
            // 生成带地理信息标签的 metrics
            output.push_str(&format!(
                "ip_traffic_tx_bytes_total{{remote_ip=\"{}\",country=\"{}\",province=\"{}\",city=\"{}\",isp=\"{}\"}} {}\n",
                ip, 
                escape_label(&geo_info.country),
                escape_label(&geo_info.province),
                escape_label(&geo_info.city),
                escape_label(&geo_info.isp),
                bytes
            ));
        }
    }
    
    Ok(output)
}

// 转义 Prometheus 标签值中的特殊字符
fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

async fn start_prometheus_server(port: u16, db_path: String, prometheus_export_threshold: i64) -> std::io::Result<()> {
    let app_state = AppState { db_path, prometheus_export_threshold };
    
    println!("启动 Prometheus Exporter 服务，监听端口: {}", port);
    println!("访问 http://localhost:{}/metrics 获取指标数据", port);
    
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(app_state.clone()))
            .route("/metrics", web::get().to(metrics_handler))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
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
#[tokio::main]
async fn main() -> Result<(), String> {
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
    
    // 如果启用了 Prometheus exporter，显示信息
    if let Some(port) = cli.prometheus_port {
        println!("Prometheus Exporter: http://0.0.0.0:{}/metrics", port);
    }
    println!("========================================");

    // 设置 Ctrl+C 信号处理
    ctrlc::set_handler(|| {
        println!("\n收到退出信号，正在优雅关闭...");
        RUNNING.store(false, Ordering::SeqCst);
    }).map_err(|e| format!("设置 Ctrl+C 处理器失败: {}", e))?;

    // 初始化数据库
    std::env::set_var("DATABASE_URL", &cli.db_path);
    
    // 初始化 GeoIP 数据库（如果提供）
    if let Some(ref geoip_path) = cli.geoip_db {
        match init_geoip_db(geoip_path) {
            Ok(_) => {},
            Err(e) => {
                eprintln!("警告: {}", e);
                eprintln!("提示: 可以从 https://dev.maxmind.com/geoip/geolite2-free-geolocation-data 下载免费的 GeoLite2-City.mmdb");
            }
        }
    } else {
        println!("未指定 GeoIP 数据库，将不包含地理位置信息");
        println!("提示: 使用 -g 参数指定 GeoIP2 数据库文件");
    }
    
    // 如果启用了 Prometheus exporter，在独立线程启动 HTTP 服务器
    if let Some(port) = cli.prometheus_port {
        let db_path_clone = cli.db_path.clone();
        let prometheus_export_threshold = cli.prometheus_export_threshold;
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if let Err(e) = start_prometheus_server(port, db_path_clone, prometheus_export_threshold).await {
                    eprintln!("Prometheus exporter 启动失败: {}", e);
                }
            });
        });
        // 给服务器一点时间启动
        thread::sleep(Duration::from_millis(500));
    }
    
    // 直接在主线程运行监控逻辑
    let mut conn = SqliteConnection::establish(&cli.db_path)
        .map_err(|e| format!("数据库连接失败: {}", e))?;
    
    if is_permanent {
        // 永久运行模式
        let mut cycle = 1;
        while RUNNING.load(Ordering::SeqCst) {
            run_monitor_cycle(&cli.iface, cli.sample_interval, &mut conn, &format!("周期 {}", cycle))?;
            cycle += 1;
        }
        println!("监控已停止，数据已保存到 {}", cli.db_path);
    } else {
        // 定时运行模式
        let cycles = cli.duration / cli.sample_interval;
        
        for cycle in 1..=cycles {
            if !RUNNING.load(Ordering::SeqCst) {
                println!("\n监控提前终止");
                break;
            }
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
