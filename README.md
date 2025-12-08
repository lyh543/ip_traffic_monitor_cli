# ip_traffic_monitor_cli

基于 iftop 的精确 IP 流量统计工具，支持 Prometheus 监控集成。

## 功能特性

- ✅ 基于 iftop 的精确流量监控
- ✅ SQLite 数据库存储历史数据
- ✅ Prometheus Exporter 接口
- ✅ 支持永久运行模式
- ✅ 自动关联进程 PID

## Prometheus Exporter 使用

### 启动监控并启用 Prometheus exporter

```bash
# 启用 Prometheus exporter，监听在 9090 端口
sudo ./target/debug/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090

# 或指定自定义端口
sudo ./target/debug/ip_traffic_monitor_cli -i eth0 -d 0 -p 8080
```

### 访问 metrics 端点

```bash
# 查看 Prometheus 格式的指标
curl http://localhost:9090/metrics
```

### Metrics 输出示例

```
# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address
# TYPE ip_traffic_tx_bytes_total counter
ip_traffic_tx_bytes_total{remote_ip="1.2.3.4"} 1048576
ip_traffic_tx_bytes_total{remote_ip="5.6.7.8"} 2097152
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

## 命令行参数

```
-i, --iface <IFACE>              出口网卡名（必填）
-d, --duration <DURATION>        监控时长（秒，0=永久运行）[默认: 30]
-f, --db-path <DB_PATH>          数据库文件路径 [默认: ip_traffic_stats_orm.db]
-s, --sample-interval <SECONDS>  iftop 采样间隔 [默认: 2]
-p, --prometheus-port <PORT>     启用 Prometheus exporter 监听端口
```

## TODO

- [ ] 统计入口流量
- [ ] 实现 SQL 查询
- [x] 接入 prometheus
- [ ] 使用 systemd 管理进程