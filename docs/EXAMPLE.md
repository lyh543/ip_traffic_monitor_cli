# 使用示例

## 示例 1：基础流量监控

```bash
# 监控 eth0 网卡，持续 60 秒
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 60

# 输出示例：
基于 iftop 的精确IP流量监控工具
网卡: eth0, 监控时长: 60秒, 采样间隔: 2秒
数据库: ip_traffic_stats_orm.db
========================================
[1/30] 正在采集流量数据...
[14:30:25] 流量统计：
  IP: 8.8.8.8 | 出口字节: 1.23 KB | 入口字节: 4.56 KB | PID: 12345
  IP: 1.1.1.1 | 出口字节: 789 B | 入口字节: 2.34 KB | PID: 0
...
监控完成，数据已保存到 ip_traffic_stats_orm.db
```

## 示例 2：启动 Prometheus Exporter（无地理信息）

```bash
# 启动监控并开启 Prometheus exporter
sudo ./target/release/ip_traffic_monitor_cli -i eth0 -d 0 -p 9090

# 在另一个终端测试
curl http://localhost:9090/metrics
```

输出：
```
# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address
# TYPE ip_traffic_tx_bytes_total counter
ip_traffic_tx_bytes_total{remote_ip="8.8.8.8",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 125440
ip_traffic_tx_bytes_total{remote_ip="1.1.1.1",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 78900
ip_traffic_tx_bytes_total{remote_ip="114.114.114.114",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 234567
```

## 示例 3：启用 GeoIP 地理位置查询

```bash
# 首先下载 GeoIP 数据库
# 从 https://www.maxmind.com/en/geolite2/signup 注册并下载

# 启动监控（含地理位置）
sudo ./target/release/ip_traffic_monitor_cli \
  -i eth0 \
  -d 0 \
  -p 9090 \
  -g ./GeoLite2-City.mmdb

# 测试 metrics
curl http://localhost:9090/metrics
```

输出（含地理信息）：
```
# HELP ip_traffic_tx_bytes_total Total transmitted bytes per IP address
# TYPE ip_traffic_tx_bytes_total counter
ip_traffic_tx_bytes_total{remote_ip="8.8.8.8",country="美国",province="加利福尼亚州",city="芒廷维尤",isp="Unknown"} 125440
ip_traffic_tx_bytes_total{remote_ip="1.1.1.1",country="澳大利亚",province="新南威尔士州",city="悉尼",isp="Unknown"} 78900
ip_traffic_tx_bytes_total{remote_ip="114.114.114.114",country="中国",province="江苏省",city="南京市",isp="Unknown"} 234567
ip_traffic_tx_bytes_total{remote_ip="223.5.5.5",country="中国",province="浙江省",city="杭州市",isp="Unknown"} 456789
```

## 示例 4：Prometheus 配置

### prometheus.yml 配置

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9090']

  - job_name: 'ip_traffic_monitor'
    static_configs:
      - targets: ['localhost:9090']  # 修改为实际的 exporter 端口
    scrape_interval: 30s
    scrape_timeout: 10s
```

### 启动 Prometheus

```bash
# 启动 Prometheus
prometheus --config.file=prometheus.yml

# 访问 Prometheus Web UI
# http://localhost:9090
```

### 常用查询

在 Prometheus 表达式浏览器中输入：

```promql
# 1. 查看所有 IP 的总流量
ip_traffic_tx_bytes_total

# 2. 按国家统计总流量
sum by (country) (ip_traffic_tx_bytes_total)

# 3. 按省份统计总流量
sum by (province) (ip_traffic_tx_bytes_total)

# 4. 按城市统计总流量
sum by (city) (ip_traffic_tx_bytes_total)

# 5. 查看中国区域的流量
ip_traffic_tx_bytes_total{country="中国"}

# 6. 查看流量 Top 10 的 IP
topk(10, ip_traffic_tx_bytes_total)

# 7. 计算流量增长率（每秒字节数）
rate(ip_traffic_tx_bytes_total[5m])

# 8. 按国家统计增长率
sum by (country) (rate(ip_traffic_tx_bytes_total[5m]))
```

## 示例 5：Grafana 仪表板

### 添加数据源

1. 打开 Grafana (http://localhost:3000)
2. Configuration → Data Sources → Add data source
3. 选择 Prometheus
4. URL: http://localhost:9090
5. 点击 "Save & Test"

### 创建面板

#### 面板 1: 按国家流量分布（饼图）

```promql
sum by (country) (ip_traffic_tx_bytes_total)
```

可视化类型：Pie Chart

#### 面板 2: 流量趋势（时间序列）

```promql
sum(rate(ip_traffic_tx_bytes_total[5m]))
```

可视化类型：Time Series

#### 面板 3: Top 10 IP（表格）

```promql
topk(10, ip_traffic_tx_bytes_total)
```

可视化类型：Table

转换：
- Format as: Table
- 显示列：remote_ip, country, province, city, value

#### 面板 4: 中国各省流量分布（柱状图）

```promql
sum by (province) (ip_traffic_tx_bytes_total{country="中国"})
```

可视化类型：Bar Chart

## 示例 6：结合 systemd 自动启动

创建 systemd 服务文件：

```bash
sudo nano /etc/systemd/system/ip-traffic-monitor.service
```

内容：

```ini
[Unit]
Description=IP Traffic Monitor with Prometheus Exporter
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/ip_traffic_monitor
ExecStart=/opt/ip_traffic_monitor/ip_traffic_monitor_cli \
  -i eth0 \
  -d 0 \
  -p 9090 \
  -g /opt/ip_traffic_monitor/GeoLite2-City.mmdb \
  -f /var/lib/ip_traffic_monitor/traffic.db
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

启用并启动服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable ip-traffic-monitor
sudo systemctl start ip-traffic-monitor
sudo systemctl status ip-traffic-monitor
```

查看日志：

```bash
sudo journalctl -u ip-traffic-monitor -f
```

## 示例 7：告警配置

### Prometheus 告警规则

创建 `alert_rules.yml`：

```yaml
groups:
  - name: ip_traffic_alerts
    interval: 30s
    rules:
      # 流量异常高的 IP
      - alert: HighTrafficIP
        expr: rate(ip_traffic_tx_bytes_total[5m]) > 10485760  # 10MB/s
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "IP {{ $labels.remote_ip }} 流量异常"
          description: "IP {{ $labels.remote_ip }} ({{ $labels.country }}/{{ $labels.city }}) 流量达到 {{ $value | humanize }}B/s"

      # 来自特定国家的流量过高
      - alert: HighTrafficFromCountry
        expr: sum by (country) (rate(ip_traffic_tx_bytes_total[5m])) > 52428800  # 50MB/s
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "来自 {{ $labels.country }} 的流量异常"
          description: "来自 {{ $labels.country }} 的总流量达到 {{ $value | humanize }}B/s"

      # 新 IP 检测
      - alert: NewIPDetected
        expr: changes(ip_traffic_tx_bytes_total[10m]) > 0 and ip_traffic_tx_bytes_total < 1048576
        labels:
          severity: info
        annotations:
          summary: "检测到新 IP: {{ $labels.remote_ip }}"
          description: "新 IP {{ $labels.remote_ip }} 来自 {{ $labels.country }}/{{ $labels.city }}"
```

在 `prometheus.yml` 中引用：

```yaml
rule_files:
  - "alert_rules.yml"
```

## 示例 8：数据导出和分析

### 导出数据

```bash
# 使用 sqlite3 查询数据库
sqlite3 ip_traffic_stats_orm.db

# 查看所有记录
SELECT * FROM ip_traffic ORDER BY timestamp DESC LIMIT 10;

# 按 IP 统计总流量
SELECT remote_ip, SUM(tx_bytes) as total_bytes 
FROM ip_traffic 
GROUP BY remote_ip 
ORDER BY total_bytes DESC 
LIMIT 10;

# 导出为 CSV
sqlite3 -header -csv ip_traffic_stats_orm.db \
  "SELECT remote_ip, SUM(tx_bytes) as total_bytes FROM ip_traffic GROUP BY remote_ip;" \
  > traffic_summary.csv
```

### Python 分析脚本示例

```python
import sqlite3
import pandas as pd
import matplotlib.pyplot as plt

# 连接数据库
conn = sqlite3.connect('ip_traffic_stats_orm.db')

# 读取数据
df = pd.read_sql_query("""
    SELECT remote_ip, SUM(tx_bytes) as total_bytes 
    FROM ip_traffic 
    GROUP BY remote_ip 
    ORDER BY total_bytes DESC 
    LIMIT 10
""", conn)

# 绘制图表
plt.figure(figsize=(12, 6))
plt.bar(df['remote_ip'], df['total_bytes'])
plt.xlabel('IP Address')
plt.ylabel('Total Bytes')
plt.title('Top 10 IPs by Traffic')
plt.xticks(rotation=45)
plt.tight_layout()
plt.savefig('traffic_top10.png')

conn.close()
```

## 性能优化建议

1. **调整采样间隔**：根据需要调整 `-s` 参数，降低 CPU 使用率
2. **数据库维护**：定期清理旧数据
3. **GeoIP 缓存**：考虑实现 IP 查询结果缓存
4. **使用 release 构建**：生产环境使用 `cargo build --release`

## 故障排查

### 问题：无法启动 iftop

```bash
# 检查 iftop 是否安装
which iftop

# 检查 sudo 权限
sudo iftop -i eth0 -t -s 2
```

### 问题：Prometheus exporter 无响应

```bash
# 检查端口是否被占用
netstat -tulnp | grep 9090

# 检查防火墙
sudo ufw status
sudo ufw allow 9090
```

### 问题：GeoIP 数据库加载失败

```bash
# 检查文件是否存在
ls -lh GeoLite2-City.mmdb

# 检查文件权限
chmod 644 GeoLite2-City.mmdb
```
