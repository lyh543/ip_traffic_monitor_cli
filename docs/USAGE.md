# IP 流量监控工具使用指南

## 功能特点

- ✅ 基于 `iftop` 的精确 IP 流量统计
- ✅ 支持定时运行和永久运行两种模式
- ✅ 自动单位转换显示 (B/s, KB/s, MB/s, GB/s)
- ✅ 数据库存储支持 (SQLite)
- ✅ 进程 PID 关联
- ✅ 实时入口和出口流量统计

## 使用方法

### 基本语法
```bash
cargo run -- --iface <网卡名> [选项]
```

### 必需参数
- `--iface <网卡名>`: 指定监控的网络接口 (如 eth0, enp2s0, wlan0)

### 可选参数
- `--duration <秒数>`: 监控时长，默认 30 秒，设置为 0 表示永久运行
- `--sample-interval <秒数>`: 采样间隔，默认 2 秒
- `--db-path <路径>`: 数据库文件路径，默认 ip_traffic_stats_orm.db

## 使用示例

### 1. 定时监控（30秒）
```bash
cargo run -- --iface enp2s0
```

### 2. 定时监控（60秒，每3秒采样）
```bash
cargo run -- --iface enp2s0 --duration 60 --sample-interval 3
```

### 3. 永久运行模式
```bash
cargo run -- --iface enp2s0 --duration 0
```

### 4. 自定义数据库路径
```bash
cargo run -- --iface enp2s0 --duration 300 --db-path ./traffic_data.db
```

## 快速测试脚本

### 定时监控测试
```bash
./quick_test.sh
```

### 永久运行测试
```bash
./test_permanent.sh
```

## 输出示例

```
基于 iftop 的精确IP流量监控工具
网卡: enp2s0, 监控时长: 30秒, 采样间隔: 2秒
数据库: ip_traffic_stats_orm.db
========================================
[1/15] 正在采集流量数据...
[15:57:09] 流量统计：
  IP: 60.162.170.223 | 出口速率: 1.36 MB/s | 入口速率: 39.00 KB/s | PID: 0
  IP: 106.14.253.168 | 出口速率: 39.88 KB/s | 入口速率: 10.99 KB/s | PID: 0
  IP: 192.168.6.102 | 出口速率: 2.12 KB/s | 入口速率: 2.56 KB/s | PID: 0
```

## 前置要求

1. 安装 `iftop`:
   ```bash
   sudo apt install iftop  # Ubuntu/Debian
   sudo yum install iftop  # CentOS/RHEL
   ```

2. 确保有 sudo 权限（iftop 需要特权访问网络接口）

3. Rust 环境已安装

## 停止监控

- **定时模式**: 自动在指定时间后停止
- **永久模式**: 按 `Ctrl+C` 停止

## 数据存储

所有流量数据自动保存到 SQLite 数据库，包含：
- 时间戳
- 远程 IP 地址
- 出口流量速率
- 关联的进程 PID