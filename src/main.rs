mod monitor;
mod iftop_monitor;
mod bpftrace_monitor;

use chrono::Local;
use clap::Parser;
use monitor::{TrafficMonitor, TrafficStats, format_bytes};
use iftop_monitor::{IftopMonitor, find_pid_for_connection};
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
// 全局 GeoIP 数据库读取器
static GEOIP_READER: Lazy<Mutex<Option<Reader<Vec<u8>>>>> = Lazy::new(|| Mutex::new(None));

// 全局退出标志
static RUNNING: AtomicBool = AtomicBool::new(true);

// 全局 IP 流量统计存储（IP -> 累计流量统计）
type IpTrafficStore = Arc<Mutex<HashMap<String, TrafficStats>>>;
static IP_TRAFFIC_STATS: Lazy<IpTrafficStore> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

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
    
    // TX 流量指标
    output.push_str("# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address\n");
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
    
    // RX 流量指标
    output.push_str("\n# HELP ip_traffic_rx_bytes_total Total received bytes per IP address\n");
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
        
        for (ip, traffic) in sorted.iter() {
            if traffic.tx_bytes > 0 || traffic.rx_bytes > 0 {
                let pid = find_pid_for_connection(ip);
                
                // 累加到全局统计
                let global_entry = global_stats.entry(ip.to_string()).or_insert_with(TrafficStats::default);
                global_entry.tx_bytes += traffic.tx_bytes;
                global_entry.rx_bytes += traffic.rx_bytes;
                global_entry.tx_packets += traffic.tx_packets;
                global_entry.rx_packets += traffic.rx_packets;
                
                println!("  IP: {} | TX: {} | RX: {} | 累计TX: {} | 累计RX: {} | PID: {}",
                       ip,
                       format_bytes(traffic.tx_bytes),
                       format_bytes(traffic.rx_bytes),
                       format_bytes(global_entry.tx_bytes),
                       format_bytes(global_entry.rx_bytes),
                       pid.unwrap_or(0));
            }
        }
    } else {
        println!("[{}] 无活跃网络连接", Local::now().format("%H:%M:%S"));
    }
    
    Ok(())
}
