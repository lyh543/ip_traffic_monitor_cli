# CPU 性能分析报告

## 发现的性能瓶颈

### 1. **GeoIP 查询性能问题** (主要瓶颈)
**位置**: `src/main.rs:109-205`

#### 问题：
每次 Prometheus `/metrics` 接口被调用时，都会对所有 IP 进行 GeoIP 查询：

```rust
for (ip, traffic) in stats.iter() {
    let geo_info = get_ip_geo_info(ip);  // 每次都查询
    // ...
}
```

虽然有缓存机制（`GEO_CACHE`），但在高流量场景下：
- 如果有大量不同的 IP，缓存未命中率高
- GeoIP 数据库查询本身就是 I/O 密集型操作
- 每个 metrics 请求都会触发锁竞争

#### 影响：
- CPU 占用高
- 响应延迟增加
- 锁竞争导致性能下降

---

### 2. **频繁的锁操作**
**位置**: `src/main.rs:224-272`, `src/bpftrace_monitor.rs:349-359`

#### 问题：
多处使用 `Mutex` 且锁的粒度较大：
- `IP_TRAFFIC_STATS` - 全局流量统计
- `GEO_CACHE` - GeoIP 缓存
- `GEOIP_READER` - GeoIP 读取器
- `stats_receiver` - bpftrace 数据接收器

在 `/metrics` 端点中，每次请求都要：
```rust
let stats = IP_TRAFFIC_STATS.lock().unwrap();  // 持有锁直到函数结束
```

---

### 3. **字符串拼接效率低**
**位置**: `src/main.rs:224-272`

#### 问题：
```rust
let mut output = String::new();
output.push_str(&format!("...")); // 多次格式化和字符串拼接
```

在高频调用场景下，大量的字符串格式化和拼接会消耗 CPU。

---

### 4. **bpftrace 输出解析**
**位置**: `src/bpftrace_monitor.rs:276-321`

#### 问题：
后台线程持续循环读取 bpftrace 输出：
```rust
loop {
    if !running.load(Ordering::SeqCst) {
        break;
    }
    match line_iter.next() { ... }
}
```

- 忙等待检查 `running` 标志
- 每行输出都要解析和处理
- 字符串操作频繁（`contains`, `find`, `parse` 等）

---

### 5. **HashMap 克隆开销**
**位置**: `src/bpftrace_monitor.rs:302`

```rust
let _ = tx.send(temp_stats.clone());  // 克隆整个 HashMap
```

每个采样周期都要克隆整个统计数据。

---

## 优化建议

### 高优先级优化

#### 1. 异步化 GeoIP 查询
```rust
// 使用异步查询和批量处理
// 或者在后台线程预先查询，避免阻塞 metrics 响应
```

#### 2. 改进缓存策略
```rust
// 使用 dashmap 或 RwLock 替代 Mutex
use dashmap::DashMap;
static GEO_CACHE: Lazy<DashMap<String, IpGeoInfo>> = Lazy::new(DashMap::new);
```

#### 3. 延迟 GeoIP 查询
```rust
// 只在 IP 首次出现时查询，而不是每次 metrics 请求
// 或者使用定时任务批量更新地理信息
```

#### 4. 使用 String 预分配
```rust
let mut output = String::with_capacity(stats.len() * 200);
```

### 中优先级优化

#### 5. 减少字符串操作
```rust
// 使用 write! 宏代替 format! + push_str
use std::fmt::Write;
write!(output, "ip_traffic_tx_bytes_total{{...}} {}\n", value)?;
```

#### 6. 优化 bpftrace 解析
```rust
// 使用更高效的解析方法
// 减少字符串分配
// 考虑使用 BytesMut 或其他零拷贝方案
```

#### 7. 避免 HashMap 克隆
```rust
// 使用 Arc 共享数据
let _ = tx.send(Arc::new(temp_stats));
```

---

## 性能测试方法

### 1. 使用 `perf` 进行 CPU profiling

```bash
# 安装 perf
sudo pacman -S perf  # Arch Linux
# 或
sudo apt install linux-tools-common linux-tools-generic  # Ubuntu

# 启动程序
sudo ./target/release/ip_traffic_monitor_cli -i enp2s0 --backend bpftrace ... &
PID=$!

# 记录性能数据（30秒）
sudo perf record -F 99 -p $PID -g -- sleep 30

# 生成报告
sudo perf report

# 生成火焰图
sudo perf script | ./FlameGraph/stackcollapse-perf.pl | ./FlameGraph/flamegraph.pl > flamegraph.svg
```

### 2. 使用 `cargo flamegraph`

```bash
# 安装
cargo install flamegraph

# 运行（需要 root）
sudo ~/.cargo/bin/cargo flamegraph --bin ip_traffic_monitor_cli -- \
    -i enp2s0 --duration 30 --backend bpftrace --geoip-db GeoLite2-City.mmdb
```

### 3. 使用 `strace` 查看系统调用

```bash
sudo strace -c -f -p $PID
```

### 4. 内存和 CPU 监控

```bash
# 实时监控
pidstat -u -r -t 1 -p $PID

# 或使用 htop
htop -p $PID
```

---

## 快速诊断步骤

1. **识别热点函数**
   ```bash
   # 运行程序并记录
   sudo perf record -F 99 -g ./target/release/ip_traffic_monitor_cli ...
   
   # 查看热点
   sudo perf report --stdio | head -50
   ```

2. **检查线程 CPU 占用**
   ```bash
   # 查看每个线程的 CPU
   top -H -p $(pgrep ip_traffic_monitor_cli)
   ```

3. **分析 Prometheus 端点性能**
   ```bash
   # 测试 metrics 响应时间
   time curl http://localhost:9091/metrics > /dev/null
   
   # 压力测试
   ab -n 100 -c 10 http://localhost:9091/metrics
   ```

4. **检查锁竞争**
   ```bash
   # 使用 perf 查看锁等待
   sudo perf record -e 'sched:sched_stat_*' -p $PID
   ```

---

## 预期性能问题原因排序

根据代码分析，CPU 占用高的最可能原因：

1. **GeoIP 数据库查询** (70%)
   - 每次 metrics 请求都查询所有 IP
   - 即使有缓存，查询本身也有开销

2. **字符串操作** (15%)
   - Prometheus metrics 格式化
   - bpftrace 输出解析

3. **锁竞争** (10%)
   - 多个线程争用全局 Mutex

4. **bpftrace 输出处理** (5%)
   - 持续的循环读取和解析

---

## 建议的优化实施顺序

1. **立即实施**：
   - 将 `GEO_CACHE` 改为 `DashMap`
   - 在 `get_ip_traffic_metrics` 中使用 `String::with_capacity`

2. **短期实施**（1-2天）：
   - 优化 GeoIP 查询逻辑，延迟查询
   - 使用 `write!` 宏代替字符串拼接

3. **中期实施**（1周）：
   - 重构为异步架构
   - 优化 bpftrace 解析器

4. **长期优化**：
   - 考虑使用更高效的 GeoIP 库
   - 实现更智能的缓存策略
