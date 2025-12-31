# 流量方向说明

## 概述

本工具已完整支持上行流量（TX）和下行流量（RX）的统计。本文档说明流量统计的原理和改进。

## 流量方向定义

### 上行流量 (TX - Transmit/Upload/Egress)
- **定义**: 从本机发送到远程 IP 的数据流量
- **监控方式**:
  - **bpftrace**: 使用 `tracepoint:net:net_dev_start_xmit` 跟踪点，统计目标 IP (`daddr`)
  - **iftop**: 解析 `=>` 符号后的速率数据

### 下行流量 (RX - Receive/Download/Ingress)
- **定义**: 从远程 IP 接收到本机的数据流量
- **监控方式**:
  - **bpftrace**: 使用以下跟踪点：
    - `tracepoint:net:netif_receive_skb` - 主要接收路径
    - `tracepoint:net:netif_rx` - 补充接收路径（某些驱动使用）
  - **iftop**: 解析 `<=` 符号后的速率数据

## 代码改进

### 1. bpftrace 脚本增强 (v2024-12-30)

增加了 `netif_rx` 跟踪点，以捕获某些网络驱动直接调用的接收路径：

```bpftrace
// 补充监控点：netif_rx（接收数据包的另一个入口点）
tracepoint:net:netif_rx
{
    $skb = (struct sk_buff *)args->skbaddr;
    $iph = (struct iphdr *)($skb->head + $skb->network_header);
    $saddr = $iph->saddr;
    $len = args->len;
    
    @rx_bytes[$saddr] = sum($len);
}
```

### 2. Prometheus Metrics 说明优化

更新了 metrics 的 HELP 描述，明确说明流量方向：

```prometheus
# HELP ip_traffic_tx_bytes_total Total transmitted bytes to remote IP address (egress/upload traffic)
# TYPE ip_traffic_tx_bytes_total counter

# HELP ip_traffic_rx_bytes_total Total received bytes from remote IP address (ingress/download traffic)
# TYPE ip_traffic_rx_bytes_total counter
```

### 3. 控制台输出优化

在输出中明确标注流量方向：

```
IP: 1.2.3.4 | TX(上行): 10.5 MB | RX(下行): 25.3 MB | 累计TX: 100.2 MB | 累计RX: 250.6 MB | PID: 1234
```

## 数据流向示意

```
远程主机 (1.2.3.4)
    ↑ TX (上行) - 发送到远程
    |
本地主机
    |
    ↓ RX (下行) - 从远程接收
远程主机 (1.2.3.4)
```

## 使用示例

### bpftrace 模式
```bash
sudo ./ip_traffic_monitor_cli -b bpftrace -s 5 -d 60
```

### iftop 模式
```bash
sudo ./ip_traffic_monitor_cli -b iftop -i eth0 -s 5 -d 60
```

### Prometheus 导出
```bash
sudo ./ip_traffic_monitor_cli -b bpftrace -d 0 -p 9091
```

访问 `http://localhost:9091/metrics` 可查看：
- `ip_traffic_tx_bytes_total{remote_ip="1.2.3.4"}` - 上行流量
- `ip_traffic_rx_bytes_total{remote_ip="1.2.3.4"}` - 下行流量

## 验证方法

### 1. 使用 curl 测试下行流量
```bash
# 下载大文件，观察下行流量增加
curl -o /dev/null http://example.com/large-file.bin
```

### 2. 使用 dd + nc 测试上行流量
```bash
# 在远程服务器监听
nc -l 9999 > /dev/null

# 本地发送数据，观察上行流量增加
dd if=/dev/zero bs=1M count=100 | nc remote-server 9999
```

### 3. 查看 Prometheus metrics
```bash
curl http://localhost:9091/metrics | grep ip_traffic
```

## 故障排查

### 下行流量为 0 或异常低

**可能原因：**
1. 网络驱动使用了不同的接收路径
2. 某些协议（如 UDP）可能需要额外的跟踪点
3. 防火墙或 iptables 规则可能影响数据包路径

**排查方法：**
```bash
# 查看可用的网络跟踪点
sudo bpftrace -l 'tracepoint:net:*'

# 手动运行 bpftrace 脚本并观察输出
sudo bpftrace /tmp/ip_traffic_monitor_bpftrace.bt
```

### iftop 模式下流量不准确

**可能原因：**
1. 网卡名称错误
2. iftop 采样间隔太短

**解决方法：**
```bash
# 确认网卡名称
ip addr

# 增加采样间隔
sudo ./ip_traffic_monitor_cli -b iftop -i eth0 -s 5
```

## 参考资料

- Linux 网络协议栈 tracepoint: https://www.kernel.org/doc/Documentation/trace/tracepoints.txt
- bpftrace 参考手册: https://github.com/iovisor/bpftrace/blob/master/docs/reference_guide.md
- iftop 文档: http://www.ex-parrot.com/~pdw/iftop/
