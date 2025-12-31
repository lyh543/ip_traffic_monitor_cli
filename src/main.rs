mod monitor;
mod iftop_monitor;
mod bpftrace_monitor;

use chrono::Local;
use clap::Parser;
use monitor::{TrafficMonitor, TrafficStats, format_bytes};
use iftop_monitor::{IftopMonitor};
use bpftrace_monitor::BpftraceMonitor;
use std::thread;
use std::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;
use actix_web::{web, App, HttpServer, HttpResponse, middleware::Compress};
use maxminddb::{geoip2, Reader};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

// ==================== 权限检查 ====================
fn check_root_permission() -> Result<(), String> {
    let is_root = unsafe { libc::geteuid() } == 0;
    
    if !is_root {
        return Err("此程序需要 root 权限运行，请使用 sudo 执行".to_string());
    }
    
    Ok(())
}

// ==================== 命令行参数定义 ====================
#[derive(Parser, Debug)]
#[command(author, version, about = "IP 流量统计工具（支持 iftop 和 bpftrace）", long_about = None)]
struct Cli {
    /// 监控后端（iftop 或 bpftrace）
    #[arg(short = 'b', long, default_value = "iftop", help = "监控后端: iftop 或 bpftrace")]
    backend: String,

    /// 出口网卡名（iftop 模式必填，通过 ip addr 查看）
    #[arg(short, long, help = "示例：eth0、ens33、enp2s0")]
    iface: Option<String>,

    /// 监控时长（单位：秒，默认 30 秒，设置为 0 表示永久运行）
    #[arg(short, long, default_value_t = 30, help = "示例：60（监控 1 分钟），0（永久运行）")]
    duration: u32,

    /// 采样间隔（默认 2 秒）
    #[arg(short = 's', long, default_value_t = 2, help = "采样间隔")]
    sample_interval: u32,

    /// 启用 Prometheus exporter（默认端口：9090）
    #[arg(short = 'p', long, help = "启用 Prometheus exporter 监听端口")]
    prometheus_port: Option<u16>,

    /// GeoIP2 数据库文件路径（可选，用于 IP 地理位置查询）
    #[arg(short = 'g', long, help = "GeoIP2 City 数据库文件路径，例如：GeoLite2-City.mmdb")]
    geoip_db: Option<String>,

    /// Prometheus metrics 流量阈值（单位：字节，默认 1MB）
    #[arg(short = 't', long, default_value_t = 1024 * 1024, help = "低于此阈值的流量不会导出到 Prometheus")]
    prometheus_export_threshold: u64,

    /// 自定义 bpftrace 脚本路径（仅 bpftrace 模式）
    #[arg(long, help = "自定义 bpftrace 脚本文件路径")]
    bpftrace_script: Option<String>,
}

// ==================== Prometheus Exporter 相关 ====================
// 全局 GeoIP 数据库读取器（使用 mmap 减少内存占用）
static GEOIP_READER: Lazy<Mutex<Option<Reader<memmap2::Mmap>>>> = Lazy::new(|| Mutex::new(None));

// 全局退出标志
static RUNNING: AtomicBool = AtomicBool::new(true);

// 全局 IP 流量统计存储（IP -> 累计流量统计）
type IpTrafficStore = Arc<Mutex<HashMap<String, TrafficStats>>>;
static IP_TRAFFIC_STATS: Lazy<IpTrafficStore> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

// IP 地理信息缓存（减少重复查询 GeoIP 数据库）
static GEO_CACHE: Lazy<Mutex<HashMap<String, IpGeoInfo>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// IP -> PID 缓存（减少 /proc 遍历），带时间戳实现 1 小时过期
static PID_CACHE: Lazy<Mutex<HashMap<String, (Option<i32>, std::time::Instant)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// PID -> 进程名缓存（减少 /proc 文件读取）
static PROCESS_NAME_CACHE: Lazy<Mutex<HashMap<i32, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// /proc/net/tcp 缓存（减少文件读取）
static TCP_CONNECTIONS_CACHE: Lazy<Mutex<(std::time::Instant, HashMap<String, u32>)>> = Lazy::new(|| {
    Mutex::new((std::time::Instant::now(), HashMap::new()))
});

// IP 地理信息结构
#[derive(Debug, Clone)]
struct IpGeoInfo {
    country: String,
    province: String,
    city: String,
    isp: String,
}

fn init_geoip_db(db_path: &str) -> Result<(), String> {
    use std::fs::File;
    
    // 使用 mmap 方式加载，大幅减少内存占用（按需加载页面）
    let file = File::open(db_path)
        .map_err(|e| format!("无法打开 GeoIP 数据库文件: {}", e))?;
    
    let mmap = unsafe { memmap2::Mmap::map(&file) }
        .map_err(|e| format!("无法映射 GeoIP 数据库文件: {}", e))?;
    
    let reader = Reader::from_source(mmap)
        .map_err(|e| format!("GeoIP 数据库加载失败: {}", e))?;
    
    *GEOIP_READER.lock().unwrap() = Some(reader);
    println!("GeoIP 数据库加载成功（使用 mmap）: {}", db_path);
    Ok(())
}

fn get_ip_geo_info(ip_str: &str) -> IpGeoInfo {
    // 先检查缓存
    {
        let cache = GEO_CACHE.lock().unwrap();
        if let Some(info) = cache.get(ip_str) {
            return info.clone();
        }
    }
    
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
    let info = match reader.lookup::<geoip2::City>(ip) {
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
        Err(_) => default_info.clone(),
    };
    
    // 保存到缓存
    {
        let mut cache = GEO_CACHE.lock().unwrap();
        cache.insert(ip_str.to_string(), info.clone());
    }
    
    info
}

#[derive(Clone)]
struct AppState {
    prometheus_export_threshold: u64,
}

async fn metrics_handler(data: web::Data<AppState>) -> HttpResponse {
    let prometheus_export_threshold = data.prometheus_export_threshold;
    
    match get_ip_traffic_metrics(prometheus_export_threshold) {
        Ok(metrics) => HttpResponse::Ok()
            .content_type("text/plain; version=0.0.4")
            .body(metrics),
        Err(e) => HttpResponse::InternalServerError()
            .body(format!("Error generating metrics: {}", e)),
    }
}

fn get_ip_traffic_metrics(prometheus_export_threshold: u64) -> Result<String, String> {
    let stats = IP_TRAFFIC_STATS.lock().unwrap();
    
    let mut output = String::new();
    
    // TX 流量指标（上行流量：本机发送到远程IP的字节数）
    output.push_str("# HELP ip_traffic_tx_bytes_total Total transmitted bytes to remote IP address (egress/upload traffic)\n");
    output.push_str("# TYPE ip_traffic_tx_bytes_total counter\n");
    
    for (ip, traffic) in stats.iter() {
        if traffic.tx_bytes <= prometheus_export_threshold {
            continue;
        }
        let geo_info = get_ip_geo_info(ip);
        
        output.push_str(&format!(
            "ip_traffic_tx_bytes_total{{remote_ip=\"{}\",country=\"{}\",province=\"{}\",city=\"{}\",isp=\"{}\"}} {}\n",
            ip, 
            escape_label(&geo_info.country),
            escape_label(&geo_info.province),
            escape_label(&geo_info.city),
            escape_label(&geo_info.isp),
            traffic.tx_bytes
        ));
    }
    
    // RX 流量指标（下行流量：从远程IP接收到本机的字节数）
    output.push_str("\n# HELP ip_traffic_rx_bytes_total Total received bytes from remote IP address (ingress/download traffic)\n");
    output.push_str("# TYPE ip_traffic_rx_bytes_total counter\n");
    
    for (ip, traffic) in stats.iter() {
        if traffic.rx_bytes <= prometheus_export_threshold {
            continue;
        }
        let geo_info = get_ip_geo_info(ip);
        
        output.push_str(&format!(
            "ip_traffic_rx_bytes_total{{remote_ip=\"{}\",country=\"{}\",province=\"{}\",city=\"{}\",isp=\"{}\"}} {}\n",
            ip, 
            escape_label(&geo_info.country),
            escape_label(&geo_info.province),
            escape_label(&geo_info.city),
            escape_label(&geo_info.isp),
            traffic.rx_bytes
        ));
    }
    
    Ok(output)
}

// 转义 Prometheus 标签值中的特殊字符
fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

async fn start_prometheus_server(port: u16, prometheus_export_threshold: u64) -> std::io::Result<()> {
    let app_state = AppState { prometheus_export_threshold };
    
    println!("启动 Prometheus Exporter 服务，监听端口: {}", port);
    println!("访问 http://localhost:{}/metrics 获取指标数据", port);
    
    HttpServer::new(move || {
        App::new()
            .wrap(Compress::default())
            .app_data(web::Data::new(app_state.clone()))
            .route("/metrics", web::get().to(metrics_handler))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

// ==================== 执行单次监控周期 ====================
fn run_monitor_cycle(monitor: &mut Box<dyn TrafficMonitor>, cycle_info: &str) -> Result<(), String> {
    println!("[{}] 正在采集流量数据...", cycle_info);
    
    match monitor.start() {
        Ok(stats) => {
            process_connections(&stats)?;
        }
        Err(e) => {
            eprintln!("监控执行失败: {}", e);
        }
    }
    
    Ok(())
}

// ==================== 带缓存的 PID 查询 ====================
fn get_pid_for_ip(ip: &str) -> Option<i32> {
    // 先检查 PID 缓存（1 小时有效期）
    {
        let mut cache = PID_CACHE.lock().unwrap();
        if let Some((cached_pid, timestamp)) = cache.get(ip) {
            // 检查缓存是否过期（1 小时 = 3600 秒）
            if timestamp.elapsed().as_secs() < 3600 {
                return *cached_pid;
            } else {
                // 缓存过期，移除旧数据
                cache.remove(ip);
            }
        }
    }
    
    // 更新 TCP 连接缓存（每 5 秒刷新一次）
    let inode = {
        let mut tcp_cache = TCP_CONNECTIONS_CACHE.lock().unwrap();
        let now = std::time::Instant::now();
        
        // 如果缓存超过 5 秒，重新读取
        if now.duration_since(tcp_cache.0).as_secs() >= 5 {
            tcp_cache.1 = build_ip_to_inode_map();
            tcp_cache.0 = now;
        }
        
        tcp_cache.1.get(ip).copied()
    };
    
    // 如果找到 inode，查询 PID
    let pid = if let Some(inode) = inode {
        find_pid_by_inode(inode).map(|p| p as i32)
    } else {
        None
    };
    
    // 保存到缓存，带时间戳
    {
        let mut cache = PID_CACHE.lock().unwrap();
        cache.insert(ip.to_string(), (pid, std::time::Instant::now()));
    }
    
    pid
}

// 批量读取 /proc/net/tcp，建立 IP -> inode 映射
fn build_ip_to_inode_map() -> HashMap<String, u32> {
    use std::net::Ipv4Addr;
    
    let mut map = HashMap::new();
    
    if let Ok(content) = std::fs::read_to_string("/proc/net/tcp") {
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 10 {
                // 解析远程地址
                if let Some(remote_addr) = parts.get(2) {
                    if let Some(addr_part) = remote_addr.split(':').next() {
                        // 将十六进制地址转换为 IP
                        if let Ok(addr_num) = u32::from_str_radix(addr_part, 16) {
                            let octets = [
                                (addr_num & 0xFF) as u8,
                                ((addr_num >> 8) & 0xFF) as u8,
                                ((addr_num >> 16) & 0xFF) as u8,
                                ((addr_num >> 24) & 0xFF) as u8,
                            ];
                            let ip = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
                            
                            // 解析 inode
                            if let Ok(inode) = parts[9].parse::<u32>() {
                                map.insert(ip.to_string(), inode);
                            }
                        }
                    }
                }
            }
        }
    }
    
    map
}

// 通过 inode 查找 PID（从 iftop_monitor.rs 移到这里）
fn find_pid_by_inode(inode: u32) -> Option<u32> {
    use procfs::process::Process;
    
    std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            Process::new(pid as i32)
                .ok()?
                .fd()
                .ok()?
                .flatten()
                .find_map(|fd| match &fd.target {
                    procfs::process::FDTarget::Socket(socket_inode) => {
                        if *socket_inode == inode as u64 {
                            Some(pid)
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
        })
}

// 根据 PID 获取进程名称（带缓存）
fn get_process_name(pid: i32) -> Option<String> {
    // 先检查缓存
    {
        let cache = PROCESS_NAME_CACHE.lock().unwrap();
        if let Some(name) = cache.get(&pid) {
            return Some(name.clone());
        }
    }
    
    // 从 /proc 读取进程名
    use procfs::process::Process;
    let process_name = Process::new(pid)
        .ok()?
        .stat()
        .ok()
        .map(|stat| stat.comm)?;
    
    // 保存到缓存
    {
        let mut cache = PROCESS_NAME_CACHE.lock().unwrap();
        cache.insert(pid, process_name.clone());
    }
    
    Some(process_name)
}

// ==================== 主函数 ====================
#[tokio::main]
async fn main() -> Result<(), String> {
    let cli = Cli::parse();
    
    let is_permanent = cli.duration == 0;
    
    // 创建监控器
    let mut monitor: Box<dyn TrafficMonitor> = match cli.backend.to_lowercase().as_str() {
        "iftop" => {
            let iface = cli.iface.clone().ok_or("iftop 模式需要指定网卡（-i 参数）")?;
            Box::new(IftopMonitor::new(iface.clone(), cli.sample_interval))
        }
        "bpftrace" => {
            Box::new(BpftraceMonitor::new(cli.sample_interval, cli.bpftrace_script.clone()))
        }
        _ => {
            return Err(format!("不支持的后端: {}，请使用 iftop 或 bpftrace", cli.backend));
        }
    };
    
    println!("IP 流量监控工具（后端: {}）", monitor.name());
    if is_permanent {
        println!("监控模式: 永久运行, 采样间隔: {}秒", cli.sample_interval);
        println!("提示: 按 Ctrl+C 停止监控");
    } else {
        println!("监控时长: {}秒, 采样间隔: {}秒", cli.duration, cli.sample_interval);
    }
    
    if let Some(port) = cli.prometheus_port {
        println!("Prometheus Exporter: http://0.0.0.0:{}/metrics", port);
    }
    println!("========================================");

    // 检查 root 权限
    check_root_permission()?;

    // 初始化监控器
    monitor.init().map_err(|e| e.to_string())?;

    // 设置 Ctrl+C 信号处理
    ctrlc::set_handler(|| {
        println!("\n收到退出信号，正在优雅关闭...");
        RUNNING.store(false, Ordering::SeqCst);
    }).map_err(|e| format!("设置 Ctrl+C 处理器失败: {}", e))?;
    
    // 初始化 GeoIP 数据库
    if let Some(ref geoip_path) = cli.geoip_db {
        match init_geoip_db(geoip_path) {
            Ok(_) => {},
            Err(e) => {
                eprintln!("警告: {}", e);
            }
        }
    } else {
        println!("未指定 GeoIP 数据库，将不包含地理位置信息");
    }
    
    // 启动 Prometheus exporter
    if let Some(port) = cli.prometheus_port {
        let prometheus_export_threshold = cli.prometheus_export_threshold;
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if let Err(e) = start_prometheus_server(port, prometheus_export_threshold).await {
                    eprintln!("Prometheus exporter 启动失败: {}", e);
                }
            });
        });
        thread::sleep(Duration::from_millis(500));
    }
    
    // 运行监控逻辑
    if is_permanent {
        let mut cycle = 1;
        while RUNNING.load(Ordering::SeqCst) {
            run_monitor_cycle(&mut monitor, &format!("周期 {}", cycle))?;
            cycle += 1;
        }
        println!("监控已停止");
    } else {
        let cycles = cli.duration / cli.sample_interval;
        
        for cycle in 1..=cycles {
            if !RUNNING.load(Ordering::SeqCst) {
                println!("\n监控提前终止");
                break;
            }
            run_monitor_cycle(&mut monitor, &format!("{}/{}", cycle, cycles))?;
        }
        
        println!("监控完成");
    }
    
    // 停止监控器
    monitor.stop().map_err(|e| e.to_string())?;
    
    Ok(())
}

// ==================== 处理连接数据的辅助函数 ====================
fn process_connections(connections: &HashMap<String, TrafficStats>) -> Result<(), String> {
    if !connections.is_empty() {
        println!("[{}] 流量统计：", Local::now().format("%H:%M:%S"));
        
        // 获取全局统计存储的锁
        let mut global_stats = IP_TRAFFIC_STATS.lock().unwrap();
        
        // 按流量排序
        let mut sorted: Vec<_> = connections.iter().collect();
        sorted.sort_by(|a, b| (b.1.tx_bytes + b.1.rx_bytes).cmp(&(a.1.tx_bytes + a.1.rx_bytes)));
        
        // 批量构建输出字符串，减少系统调用
        let mut output = String::with_capacity(sorted.len() * 100);
        
        for (ip, traffic) in sorted.iter() {
            if traffic.tx_bytes > 0 || traffic.rx_bytes > 0 {
                let pid = get_pid_for_ip(ip);
                let process_name = pid.and_then(|p| get_process_name(p));
                
                // 累加到全局统计
                let global_entry = global_stats.entry(ip.to_string()).or_insert_with(TrafficStats::default);
                global_entry.tx_bytes += traffic.tx_bytes;
                global_entry.rx_bytes += traffic.rx_bytes;
                global_entry.tx_packets += traffic.tx_packets;
                global_entry.rx_packets += traffic.rx_packets;
                
                // 添加到输出字符串
                use std::fmt::Write;
                let process_info = match (pid, process_name) {
                    (Some(p), Some(name)) => format!("{} ({})", p, name),
                    (Some(p), None) => format!("{}", p),
                    _ => "0".to_string(),
                };
                let _ = write!(output, "  IP: {} | TX(上行): {} | RX(下行): {} | 累计TX: {} | 累计RX: {} | PID: {}\n",
                       ip,
                       format_bytes(traffic.tx_bytes),
                       format_bytes(traffic.rx_bytes),
                       format_bytes(global_entry.tx_bytes),
                       format_bytes(global_entry.rx_bytes),
                       process_info);
            }
        }
        
        // 一次性输出所有内容
        print!("{}", output);
    } else {
        println!("[{}] 无活跃网络连接", Local::now().format("%H:%M:%S"));
    }
    
    Ok(())
}
