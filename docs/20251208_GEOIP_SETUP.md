# GeoIP 数据库配置指南

本工具支持使用 GeoIP2 数据库来查询 IP 地址的地理位置信息（国家、省份、城市）和运营商信息。

## 方案一：使用 MaxMind GeoLite2（免费，推荐）

### 1. 注册 MaxMind 账号

访问 https://www.maxmind.com/en/geolite2/signup 注册免费账号。

### 2. 下载 GeoLite2-City 数据库

1. 登录后访问 https://www.maxmind.com/en/accounts/current/geoip/downloads
2. 下载 **GeoLite2 City** - MMDB 格式
3. 解压得到 `GeoLite2-City.mmdb` 文件

### 3. 使用数据库

```bash
# 将数据库文件放到项目目录
cp GeoLite2-City.mmdb /path/to/ip_traffic_monitor_cli/

# 启动监控时指定数据库路径
sudo ./target/debug/ip_traffic_monitor_cli \
  -i eth0 \
  -d 0 \
  -p 9090 \
  -g GeoLite2-City.mmdb
```

### 数据库说明

- **GeoLite2-City.mmdb**: 包含国家、省份、城市信息
  - ✅ 国家信息（中英文）
  - ✅ 省份/州信息（中英文）
  - ✅ 城市信息（中英文）
  - ❌ 不包含 ISP 运营商信息

- **GeoIP2-ISP.mmdb** (付费): 包含 ISP 运营商信息
  - 如需运营商信息，可购买 GeoIP2-ISP 数据库
  - 价格参考：https://www.maxmind.com/en/geoip2-isp-database

## 方案二：使用纯真 IP 数据库（中文友好）

纯真 IP 数据库（QQWry.dat）是一个专注于中文的免费 IP 数据库，包含详细的中国区域和运营商信息。

### 优势
- ✅ 完全免费
- ✅ 中文数据详细
- ✅ 包含运营商信息（电信、联通、移动等）
- ✅ 更新频繁

### 如何使用

目前本工具仅支持 MaxMind 格式的数据库。如需使用纯真 IP 数据库，可以考虑：

1. 使用第三方转换工具将 QQWry.dat 转换为 MMDB 格式
2. 或者使用纯真 IP 的在线 API 服务

## Prometheus Metrics 输出示例

### 不使用 GeoIP 数据库

```
ip_traffic_tx_bytes_total{remote_ip="1.2.3.4",country="Unknown",province="Unknown",city="Unknown",isp="Unknown"} 1048576
```

### 使用 GeoLite2-City 数据库

```
ip_traffic_tx_bytes_total{remote_ip="8.8.8.8",country="美国",province="加利福尼亚州",city="芒廷维尤",isp="Unknown"} 2097152
ip_traffic_tx_bytes_total{remote_ip="114.114.114.114",country="中国",province="江苏省",city="南京市",isp="Unknown"} 3145728
```

### 使用 GeoIP2-ISP 数据库（需额外配置）

如果配置了 ISP 数据库，ISP 字段将显示运营商名称：

```
ip_traffic_tx_bytes_total{remote_ip="114.114.114.114",country="中国",province="江苏省",city="南京市",isp="China Telecom"} 3145728
```

## 数据库更新

MaxMind 每周更新一次 GeoLite2 数据库。建议：

1. 设置定期任务（cron）自动下载更新
2. 使用 MaxMind 的 geoipupdate 工具自动更新

```bash
# 安装 geoipupdate
sudo apt-get install geoipupdate

# 配置并运行更新
sudo geoipupdate
```

## 常见问题

### Q: 为什么 ISP 字段总是显示 "Unknown"？

A: GeoLite2-City 免费数据库不包含 ISP 信息。需要购买 GeoIP2-ISP 数据库或使用其他包含运营商信息的数据库。

### Q: 中国 IP 地址信息不准确怎么办？

A: GeoLite2 对中国的数据相对较粗。可以考虑：
- 使用纯真 IP 数据库
- 使用高德、百度等国内服务商的 IP 定位 API
- 购买 MaxMind 的付费 GeoIP2 数据库（准确度更高）

### Q: 数据库文件很大，影响性能吗？

A: 不会。数据库在启动时加载到内存，查询速度非常快（微秒级）。GeoLite2-City 约 60MB，占用内存可接受。

## 参考链接

- MaxMind GeoLite2: https://dev.maxmind.com/geoip/geolite2-free-geolocation-data
- GeoIP2 文档: https://maxmind.github.io/GeoIP2-rust/
- 纯真 IP 数据库: http://www.cz88.net/
