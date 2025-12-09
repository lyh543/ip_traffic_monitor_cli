use crate::monitor::{TrafficMonitor, TrafficStats};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// 基于 bpftrace 的流量监控器
pub struct BpftraceMonitor {
    sample_interval: u32,
    script_path: Option<String>,
    child_process: Option<Child>,
    running: Arc<AtomicBool>,
    stats_receiver: Option<Arc<Mutex<Receiver<HashMap<String, TrafficStats>>>>>,
    output_thread: Option<thread::JoinHandle<()>>,
}

impl BpftraceMonitor {
    pub fn new(sample_interval: u32, script_path: Option<String>) -> Self {
        Self {
            sample_interval,
            script_path,
            child_process: None,
            running: Arc::new(AtomicBool::new(false)),
            stats_receiver: None,
            output_thread: None,
        }
    }

    /// 生成 bpftrace 脚本
    fn generate_script(&self) -> String {
        format!(
            r#"
BEGIN {{
    printf("BPFTRACE_MONITOR_START\n");
}}

// 监控接收流量
tracepoint:net:netif_receive_skb
{{
    $skb = (struct sk_buff *)args->skbaddr;
    $iph = (struct iphdr *)($skb->head + $skb->network_header);
    $saddr = $iph->saddr;
    $len = args->len;
    
    @rx_bytes[ntop($saddr)] = sum($len);
    @rx_packets[ntop($saddr)] = count();
}}

// 监控发送流量
tracepoint:net:net_dev_start_xmit
{{
    $skb = (struct sk_buff *)args->skbaddr;
    $iph = (struct iphdr *)($skb->head + $skb->network_header);
    $daddr = $iph->daddr;
    $len = args->len;
    
    @tx_bytes[ntop($daddr)] = sum($len);
    @tx_packets[ntop($daddr)] = count();
}}

interval:s:{} {{
    printf("STATS_UPDATE\n");
    printf("TX_BYTES:\n");
    print(@tx_bytes);
    printf("TX_PACKETS:\n");
    print(@tx_packets);
    printf("RX_BYTES:\n");
    print(@rx_bytes);
    printf("RX_PACKETS:\n");
    print(@rx_packets);
    printf("STATS_END\n");
    
    clear(@tx_bytes);
    clear(@tx_packets);
    clear(@rx_bytes);
    clear(@rx_packets);
}}
"#,
            self.sample_interval
        )
    }

    /// 检查 IP 地址是否为公网 IP（过滤私有、保留、本地地址）
    fn is_valid_ip(ip: &str) -> bool {
        // 尝试解析为标准 IP 地址格式
        if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
            match addr {
                std::net::IpAddr::V4(ipv4) => {
                    let octets = ipv4.octets();
                    
                    // 过滤 0.0.0.0/8 (当前网络)
                    if octets[0] == 0 {
                        return false;
                    }
                    
                    // 过滤 10.0.0.0/8 (私有网络 A 类)
                    if octets[0] == 10 {
                        return false;
                    }
                    
                    // 过滤 127.0.0.0/8 (本地回环)
                    if octets[0] == 127 {
                        return false;
                    }
                    
                    // 过滤 172.16.0.0/12 (私有网络 B 类)
                    if octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31 {
                        return false;
                    }
                    
                    // 过滤 192.168.0.0/16 (私有网络 C 类)
                    if octets[0] == 192 && octets[1] == 168 {
                        return false;
                    }
                    
                    // 过滤 169.254.0.0/16 (链路本地地址)
                    if octets[0] == 169 && octets[1] == 254 {
                        return false;
                    }
                    
                    // 过滤 224.0.0.0/4 (组播地址)
                    if octets[0] >= 224 && octets[0] <= 239 {
                        return false;
                    }
                    
                    // 过滤 240.0.0.0/4 (保留地址)
                    if octets[0] >= 240 {
                        return false;
                    }
                    
                    // 过滤 255.255.255.255 (广播地址)
                    if octets == [255, 255, 255, 255] {
                        return false;
                    }
                    
                    // 其他地址视为公网 IP
                    true
                }
                std::net::IpAddr::V6(ipv6) => {
                    // IPv6: 过滤本地和特殊地址
                    if ipv6.is_loopback() || ipv6.is_unspecified() || ipv6.is_multicast() {
                        return false;
                    }
                    // 过滤链路本地地址 (fe80::/10)
                    let segments = ipv6.segments();
                    if segments[0] & 0xffc0 == 0xfe80 {
                        return false;
                    }
                    // 过滤唯一本地地址 (fc00::/7)
                    if segments[0] & 0xfe00 == 0xfc00 {
                        return false;
                    }
                    true
                }
            }
        } else {
            // 无法解析为 IP 地址
            false
        }
    }

    /// 解析 bpftrace 输出行（静态方法）
    fn parse_output_line(
        line: &str,
        current_section: &mut String,
        stats: &mut HashMap<String, TrafficStats>,
    ) {
        let line = line.trim();

        if line == "TX_BYTES:" {
            *current_section = "tx_bytes".to_string();
        } else if line == "TX_PACKETS:" {
            *current_section = "tx_packets".to_string();
        } else if line == "RX_BYTES:" {
            *current_section = "rx_bytes".to_string();
        } else if line == "RX_PACKETS:" {
            *current_section = "rx_packets".to_string();
        } else if line == "STATS_END" {
            *current_section = String::new();
        } else if !current_section.is_empty() && line.starts_with('@') && line.contains('[') && line.contains("]:") {
            // 解析 bpftrace map 输出格式: @map_name[key]: value
            // 例如: @tx_bytes[192.168.1.1]: 1234
            if let Some(bracket_start) = line.find('[') {
                if let Some(bracket_end) = line.find("]:") {
                    let ip = &line[bracket_start + 1..bracket_end];
                    
                    // 过滤无效 IP 地址
                    if !Self::is_valid_ip(ip) {
                        return;
                    }
                    
                    let value_str = &line[bracket_end + 2..].trim();
                    
                    if let Ok(value) = value_str.parse::<u64>() {
                        let entry = stats.entry(ip.to_string()).or_insert_with(TrafficStats::default);

                        match current_section.as_str() {
                            "tx_bytes" => entry.tx_bytes = value,
                            "tx_packets" => entry.tx_packets = value,
                            "rx_bytes" => entry.rx_bytes = value,
                            "rx_packets" => entry.rx_packets = value,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

impl TrafficMonitor for BpftraceMonitor {
    fn init(&mut self) -> Result<(), Box<dyn Error>> {
        // 检查 bpftrace 是否可用
        let output = Command::new("bpftrace").arg("--version").output();
        
        match output {
            Ok(out) => {
                let version = String::from_utf8_lossy(&out.stdout);
                println!("bpftrace 监控器初始化成功: {}", version.trim());
            }
            Err(e) => {
                return Err(format!("bpftrace 不可用: {}. 请确保已安装 bpftrace", e).into());
            }
        }

        // 启动持续运行的 bpftrace 进程
        let script = if let Some(ref path) = self.script_path {
            std::fs::read_to_string(path)?
        } else {
            self.generate_script()
        };

        // 将脚本写入临时文件
        let temp_script_path = "/tmp/ip_traffic_monitor_bpftrace.bt";
        std::fs::write(temp_script_path, &script)?;

        self.running.store(true, Ordering::SeqCst);

        let mut child = Command::new("sudo")
            .args(&["stdbuf", "-o0", "-e0", "bpftrace", "-B", "none", temp_script_path])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // bpftrace 的 printf() 输出在 stdout，诊断信息在 stderr
        let stdout = child.stdout.take()
            .ok_or("无法获取 bpftrace stdout")?;

        // 创建通道用于接收统计数据
        let (tx, rx): (Sender<HashMap<String, TrafficStats>>, Receiver<HashMap<String, TrafficStats>>) = mpsc::channel();
        self.stats_receiver = Some(Arc::new(Mutex::new(rx)));

        let running = Arc::clone(&self.running);
        
        // 启动后台线程持续读取 bpftrace 输出
        let output_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut current_section = String::new();
            let mut temp_stats: HashMap<String, TrafficStats> = HashMap::new();

            let mut line_iter = reader.lines();
            loop {
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                match line_iter.next() {
                    Some(Ok(line)) => {
                    
                    // 跳过 BPFTRACE_MONITOR_START 消息
                    if line.contains("BPFTRACE_MONITOR_START") {
                        continue;
                    }

                    if line.contains("STATS_UPDATE") {
                        temp_stats.clear();
                        continue;
                    }

                    if line.contains("STATS_END") {
                        // 发送统计数据到主线程
                        if !temp_stats.is_empty() {
                            let _ = tx.send(temp_stats.clone());
                        }
                        temp_stats.clear();
                        current_section.clear();
                        continue;
                    }

                    // 解析输出行
                    Self::parse_output_line(&line, &mut current_section, &mut temp_stats);
                    }
                    Some(Err(e)) => {
                        eprintln!("[错误] 读取 bpftrace 输出失败: {}", e);
                        break;
                    }
                    None => {
                        break;
                    }
                }
            }
        });

        self.output_thread = Some(output_thread);
        self.child_process = Some(child);

        // 等待 bpftrace 启动并附加探针
        println!("等待 bpftrace 进程启动（{} 秒）...", self.sample_interval + 1);
        std::thread::sleep(std::time::Duration::from_secs((self.sample_interval + 1) as u64));
        
        Ok(())
    }

    fn start(&mut self) -> Result<HashMap<String, TrafficStats>, Box<dyn Error>> {
        // 从通道接收最新的统计数据
        let receiver = self.stats_receiver.as_ref()
            .ok_or("stats_receiver 未初始化")?;
        
        let mut latest_stats = HashMap::new();
        
        // 清空旧数据，只保留最新的
        loop {
            let recv_guard = receiver.lock().unwrap();
            match recv_guard.try_recv() {
                Ok(stats) => {
                    drop(recv_guard);
                    latest_stats = stats;
                }
                Err(_) => {
                    drop(recv_guard);
                    break;
                }
            }
        }

        // 如果没有新数据，等待一个采样周期
        if latest_stats.is_empty() {
            use std::time::Duration;
            let timeout = Duration::from_secs((self.sample_interval + 5) as u64);
            
            let recv_guard = receiver.lock().unwrap();
            match recv_guard.recv_timeout(timeout) {
                Ok(stats) => latest_stats = stats,
                Err(e) => {
                    eprintln!("等待统计数据超时: {}，返回空数据", e);
                }
            }
        }
        
        Ok(latest_stats)
    }

    fn stop(&mut self) -> Result<(), Box<dyn Error>> {
        self.running.store(false, Ordering::SeqCst);
        
        // 等待输出线程结束
        if let Some(handle) = self.output_thread.take() {
            let _ = handle.join();
        }
        
        if let Some(mut child) = self.child_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        
        Ok(())
    }

    fn name(&self) -> &str {
        "bpftrace"
    }
}
