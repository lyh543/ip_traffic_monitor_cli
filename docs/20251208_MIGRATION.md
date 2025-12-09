# 重构迁移说明

## 变更概述

本次重构将应用从**数据库持久化模式**改为**内存存储模式**，专注于为 Prometheus 提供实时指标数据。

## 主要变更

### 1. 移除的功能
- ❌ SQLite 数据库持久化
- ❌ Diesel ORM 框架
- ❌ 历史数据存储
- ❌ 数据库相关命令行参数 (`-f, --db-path`)

### 2. 新增的功能
- ✅ 内存 HashMap 存储（`Arc<Mutex<HashMap<String, u64>>>`）
- ✅ IP 字节数累计统计
- ✅ 实时 Prometheus metrics 导出

### 3. 保留的功能
- ✅ iftop 流量监控
- ✅ Prometheus exporter
- ✅ GeoIP2 地理位置查询
- ✅ PID 进程关联
- ✅ 永久运行模式

## 架构对比

### 旧架构
```
iftop → 解析流量 → 写入 SQLite → Prometheus 读取数据库 → 导出 metrics
```

### 新架构
```
iftop → 解析流量 → 累加到内存 HashMap → Prometheus 直接读取内存 → 导出 metrics
```

## 代码变更

### 依赖变更 (Cargo.toml)
```diff
- diesel = { version = "2.1", features = ["sqlite", "chrono"] }
- diesel_migrations = "2.1"
- byteorder = "1.5.0"
```

### 核心数据结构
```rust
// 旧版：数据库模型
#[derive(Insertable)]
struct NewIpTraffic {
    timestamp: String,
    remote_ip: String,
    tx_bytes: i32,
    pid: Option<i32>,
}

// 新版：内存存储
static IP_TRAFFIC_STATS: Lazy<Arc<Mutex<HashMap<String, u64>>>> = ...;
```

### 数据处理流程
```rust
// 旧版：写入数据库
diesel::insert_into(ip_traffic::table)
    .values(&traffic)
    .execute(conn)?;

// 新版：累加到内存
let mut stats = IP_TRAFFIC_STATS.lock().unwrap();
let counter = stats.entry(remote_ip.clone()).or_insert(0);
*counter += tx_bytes as u64;
```

### Prometheus Metrics 生成
```rust
// 旧版：从数据库查询
let results = ip_traffic
    .group_by(remote_ip)
    .select((remote_ip, sum(tx_bytes)))
    .load(&mut conn)?;

// 新版：从内存读取
let stats = IP_TRAFFIC_STATS.lock().unwrap();
for (ip, &bytes) in stats.iter() {
    // 生成 metrics
}
```

## 使用方式变更

### 命令行参数
```bash
# 旧版
sudo ./ip_traffic_monitor_cli -i eth0 -d 0 -p 9090 -f ./data.db

# 新版（移除 -f 参数）
sudo ./ip_traffic_monitor_cli -i eth0 -d 0 -p 9090
```

### 数据持久化
- **旧版**：数据自动保存到 SQLite 数据库
- **新版**：数据仅在内存中，进程重启后丢失

### 推荐的数据持久化方案
使用 Prometheus 进行长期存储：

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'ip_traffic_monitor'
    static_configs:
      - targets: ['localhost:9090']
    scrape_interval: 30s  # 每 30 秒采集一次

# 数据保留配置
global:
  retention: 30d  # 保留 30 天数据
```

## 优势

1. **性能提升**：无数据库 I/O 开销
2. **架构简化**：移除数据库依赖
3. **实时性强**：内存访问速度快
4. **Prometheus 原生**：直接适配 Prometheus 的拉模型
5. **资源占用少**：无需维护数据库文件

## 注意事项

1. **数据丢失风险**：进程重启后，历史累计数据会丢失
   - 解决方案：依赖 Prometheus 进行持久化存储

2. **内存占用**：长期运行可能累积大量 IP
   - 当前无过期机制，每个 IP 仅占用约 40-60 字节
   - 10 万个 IP 约占用 4-6 MB 内存
   - 本程序基础内存占用：Rust 实现约 7MB
   - 依赖工具内存占用：iftop 约 7MB，bpftrace 约 60MB（如使用）

3. **Counter 重置**：Prometheus 需要处理 counter 重置
   - Prometheus 会自动检测并处理 counter 重置

## 迁移建议

### 如果你需要历史数据查询
使用 Prometheus + Grafana：
- Prometheus 负责数据采集和存储
- Grafana 负责可视化和查询
- 支持更强大的时序分析和告警功能

### 如果你需要离线数据处理
可以保留旧版本，或者：
- 使用 Prometheus 的 Remote Write 将数据写入其他存储
- 定期导出 Prometheus 数据进行分析

## 兼容性

- ✅ Prometheus metrics 格式完全兼容
- ✅ GeoIP2 功能保持不变
- ✅ 命令行参数大部分兼容（仅移除 -f）
- ❌ 无法读取旧版的 SQLite 数据库

## 文件清理（可选）

重构后可以删除的文件：
```bash
rm diesel.toml
rm -rf migrations/
rm ip_traffic_stats_orm.db
```

这些文件已不再使用，但保留也不影响程序运行。
