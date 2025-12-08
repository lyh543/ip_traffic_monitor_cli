# ip_traffic_monitor_cli

基于 iftop 的精确 IP 流量统计工具，支持 Prometheus 监控集成和 IP 地理位置查询。

## 功能特性

- ✅ 基于 iftop 的精确流量监控
- ✅ SQLite 数据库存储历史数据
- ✅ Prometheus Exporter 接口
- ✅ IP 地理位置信息（国家、省份、城市）
- ✅ ISP 运营商信息支持
- ✅ 支持永久运行模式
- ✅ 自动关联进程 PID

## 快速开始

### 基本使用（无地理信息）

```bash
# 编译
cargo build --release

# 启动监控和 Prometheus exporter
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090
```

### 完整使用（含地理信息）

```bash
# 1. 下载 GeoIP 数据库（见下文）
# 2. 启动监控
sudo ./target/release/ip_traffic_monitor_cli \
  -i eth0 \
  -d 0 \
  -p 9090 \
  -g GeoLite2-City.mmdb
```

## Prometheus Exporter 使用

### 启动监控并启用 Prometheus exporter

```bash
# 启用 Prometheus exporter，监听在 9090 端口
sudo ./target/debug/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090

# 或指定自定义端口
sudo ./target/debug/ip_traffic_monitor_cli -i eth0 -d 0 -p 8080

# 启用地理位置查询
sudo ./target/debug/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090 -g GeoLite2-City.mmdb
```

### 访问 metrics 端点

```bash
# 查看 Prometheus 格式的指标
curl http://localhost:9090/metrics
```

### Metrics 输出示例

#### 不使用 GeoIP 数据库

```
# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address
# TYPE ip_traffic_tx_bytes_total counter
ip_traffic_tx_bytes_total{remote_ip="1.2.3.4",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 1048576
ip_traffic_tx_bytes_total{remote_ip="5.6.7.8",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 2097152
```

#### 使用 GeoIP 数据库

```
# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address
# TYPE ip_traffic_tx_bytes_total counter
ip_traffic_tx_bytes_total{remote_ip="8.8.8.8",country="美国",province="加利福尼亚州",city="芒廷维尤",isp="Unknown"} 2097152
ip_traffic_tx_bytes_total{remote_ip="114.114.114.114",country="中国",province="江苏省",city="南京市",isp="Unknown"} 3145728
```

### Prometheus 配置

在 `prometheus.yml` 中添加：

```yaml
scrape_configs:
  - job_name: 'ip_traffic_monitor'
    static_configs:
      - targets: ['localhost:9090']
    scrape_interval: 30s
```

### Prometheus 查询示例

```promql
# 按国家统计流量
sum by (country) (ip_traffic_tx_bytes_total)

# 按省份统计流量
sum by (province) (ip_traffic_tx_bytes_total)

# 按运营商统计流量
sum by (isp) (ip_traffic_tx_bytes_total)

# 查看中国区域的流量
ip_traffic_tx_bytes_total{country="中国"}

# 流量增长率
rate(ip_traffic_tx_bytes_total[5m])
```

## GeoIP 数据库配置

### 下载 MaxMind GeoLite2 数据库（免费）

1. 注册 MaxMind 账号：https://www.maxmind.com/en/geolite2/signup
2. 下载 GeoLite2-City 数据库（MMDB 格式）
3. 解压得到 `GeoLite2-City.mmdb` 文件

详细配置说明请参考：[docs/GEOIP_SETUP.md](docs/GEOIP_SETUP.md)

### 数据库功能对比

| 数据库 | 费用 | 国家 | 省份 | 城市 | ISP |
|--------|------|------|------|------|-----|
| GeoLite2-City | 免费 | ✅ | ✅ | ✅ | ❌ |
| GeoIP2-City | 付费 | ✅ | ✅ | ✅ | ❌ |
| GeoIP2-ISP | 付费 | ❌ | ❌ | ❌ | ✅ |

注：需要同时使用 GeoIP2-City 和 GeoIP2-ISP 才能获得完整的地理和运营商信息。

## 命令行参数

```
-i, --iface <IFACE>              出口网卡名（必填）
-d, --duration <DURATION>        监控时长（秒，0=永久运行）[默认: 30]
-f, --db-path <DB_PATH>          数据库文件路径 [默认: ip_traffic_stats_orm.db]
-s, --sample-interval <SECONDS>  iftop 采样间隔 [默认: 2]
-p, --prometheus-port <PORT>     启用 Prometheus exporter 监听端口
-g, --geoip-db <PATH>            GeoIP2 数据库文件路径（可选）
```

## 使用场景

### 1. 实时流量监控

```bash
# 持续监控并保存到数据库
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 0
```

### 2. 定时监控

```bash
# 监控 5 分钟
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 300
```

### 3. Prometheus 集成

```bash
# 启动 exporter 供 Prometheus 抓取
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090 -g GeoLite2-City.mmdb
```

### 4. Grafana 可视化

1. 配置 Prometheus 数据源
2. 创建仪表板
3. 使用以下查询：
   - 按国家/地区分布：`sum by (country) (ip_traffic_tx_bytes_total)`
   - Top N 流量 IP：`topk(10, ip_traffic_tx_bytes_total)`
   - 流量趋势：`rate(ip_traffic_tx_bytes_total[5m])`

## TODO

- [ ] 统计入口流量
- [ ] 实现 SQL 查询
- [x] 接入 prometheus
- [x] IP 地理位置查询
- [ ] 使用 systemd 管理进程
- [ ] 支持 GeoIP2-ISP 数据库
- [ ] 支持纯真 IP 数据库

## 依赖

- iftop: 流量监控工具
- SQLite: 数据存储
- GeoIP2 数据库（可选）: IP 地理位置查询

## 许可证

MIT