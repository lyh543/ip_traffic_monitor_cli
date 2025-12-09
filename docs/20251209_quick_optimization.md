# 立即降低 CPU 占用的优化方案

## 快速优化（10分钟实施）

基于代码分析，以下是几个可以立即实施的优化，预计可降低 30-50% CPU 占用：

### 优化 1: 预分配字符串容量

**文件**: `src/main.rs:224`

**当前代码**:
```rust
let mut output = String::new();
```

**优化后**:
```rust
// 预估每个 IP 需要约 200 字节（2行指标 * 100字节/行）
let mut output = String::with_capacity(stats.len() * 200);
```

**效果**: 减少内存重分配，降低 5-10% CPU

---

### 优化 2: 使用 write! 宏替代 format! + push_str

**文件**: `src/main.rs:233-241` 和 `250-258`

**当前代码**:
```rust
output.push_str(&format!(
    "ip_traffic_tx_bytes_total{{remote_ip=\"{}\",country=\"{}\",province=\"{}\",city=\"{}\",isp=\"{}\"}} {}\n",
    ip, 
    escape_label(&geo_info.country),
    escape_label(&geo_info.province),
    escape_label(&geo_info.city),
    escape_label(&geo_info.isp),
    traffic.tx_bytes
));
```

**优化后**:
```rust
use std::fmt::Write;

write!(output,
    "ip_traffic_tx_bytes_total{{remote_ip=\"{}\",country=\"{}\",province=\"{}\",city=\"{}\",isp=\"{}\"}} {}\n",
    ip, 
    escape_label(&geo_info.country),
    escape_label(&geo_info.province),
    escape_label(&geo_info.city),
    escape_label(&geo_info.isp),
    traffic.tx_bytes
).unwrap();
```

**效果**: 减少字符串分配，降低 10-15% CPU

---

### 优化 3: 使用 DashMap 替代 Mutex<HashMap>

**文件**: `src/main.rs:88`

**步骤**:

1. 添加依赖到 `Cargo.toml`:
```toml
dashmap = "6.1"
```

2. 修改代码:

**当前**:
```rust
static GEO_CACHE: Lazy<Mutex<HashMap<String, IpGeoInfo>>> = Lazy::new(|| Mutex::new(HashMap::new()));
```

**优化后**:
```rust
use dashmap::DashMap;
static GEO_CACHE: Lazy<DashMap<String, IpGeoInfo>> = Lazy::new(DashMap::new);
```

3. 更新使用处:

**当前** (`src/main.rs:113-117`):
```rust
{
    let cache = GEO_CACHE.lock().unwrap();
    if let Some(info) = cache.get(ip_str) {
        return info.clone();
    }
}
```

**优化后**:
```rust
if let Some(info) = GEO_CACHE.get(ip_str) {
    return info.clone();
}
```

**当前** (`src/main.rs:194-198`):
```rust
{
    let mut cache = GEO_CACHE.lock().unwrap();
    cache.insert(ip_str.to_string(), info.clone());
}
```

**优化后**:
```rust
GEO_CACHE.insert(ip_str.to_string(), info.clone());
```

**效果**: 消除锁竞争，降低 15-20% CPU

---

### 优化 4: 延迟 GeoIP 查询（更激进）

在后台线程预先查询 GeoIP，而不是在 metrics 请求时查询。

**文件**: `src/main.rs:422-433`

在 `process_connections` 中添加 GeoIP 预查询:

```rust
fn process_connections(connections: &HashMap<String, TrafficStats>) -> Result<(), String> {
    if !connections.is_empty() {
        println!("[{}] 流量统计：", Local::now().format("%H:%M:%S"));
        
        let mut global_stats = IP_TRAFFIC_STATS.lock().unwrap();
        
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
                
                // ✨ 新增：在这里预先查询 GeoIP，缓存起来
                if GEOIP_READER.lock().unwrap().is_some() {
                    let _ = get_ip_geo_info(ip);  // 触发查询和缓存
                }
                
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
```

**效果**: metrics 请求直接从缓存读取，降低 20-30% CPU

---

## 完整优化补丁

如果要一次性应用所有优化，参考 `docs/performance_optimization_patch.md`

---

## 测试优化效果

优化前后对比：

```bash
# 优化前
./diagnose_hotspot.sh
# 记录 CPU 占用

# 应用优化
# ... 修改代码 ...

# 重新编译
cargo build --release

# 优化后
./diagnose_hotspot.sh
# 对比 CPU 占用
```

预期结果：
- 主进程 CPU 从 50-70% 降至 20-35%
- metrics 响应时间减少 50%+

---

## 如果 CPU 仍然高

如果应用所有优化后 CPU 仍然 > 40%，需要：

1. 使用 `perf` 进行热点分析
2. 检查是否是 bpftrace 本身的开销
3. 考虑切换到 libbpf-rs 直接使用 eBPF
4. 限制监控的 IP 数量

参考 `docs/performance_analysis.md` 的详细分析方法。
