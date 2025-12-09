use std::collections::HashMap;
use std::error::Error;

/// 流量统计数据结构
#[derive(Debug, Clone, Default)]
pub struct TrafficStats {
    pub tx_bytes: u64,      // 发送字节数
    pub rx_bytes: u64,      // 接收字节数
    pub tx_packets: u64,    // 发送数据包数
    pub rx_packets: u64,    // 接收数据包数
}

/// 流量监控器接口
pub trait TrafficMonitor: Send + Sync {
    /// 初始化监控器
    fn init(&mut self) -> Result<(), Box<dyn Error>>;
    
    /// 开始监控（阻塞调用）
    /// 返回每个 IP 的流量统计
    fn start(&mut self) -> Result<HashMap<String, TrafficStats>, Box<dyn Error>>;
    
    /// 停止监控
    fn stop(&mut self) -> Result<(), Box<dyn Error>>;
    
    /// 获取监控器名称
    fn name(&self) -> &str;
}

/// 格式化字节数显示
pub fn format_bytes(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.2} MB", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.2} KB", bytes / 1024.0)
    } else {
        format!("{:.0} B", bytes)
    }
}
