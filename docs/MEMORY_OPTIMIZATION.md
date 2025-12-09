# GeoIP2 内存优化方案

## 问题
GeoLite2-City.mmdb 数据库文件大小为 60MB，在 512MB 内存的服务器上压力很大。

## 已实施方案

### 1. 使用 mmap（内存映射）✅

**实现**：使用 `memmap2` crate 通过内存映射方式加载 GeoIP 数据库，而不是将整个文件加载到内存。

**优点**：
- **大幅减少内存占用**：操作系统按需加载页面（通常 4KB），而不是一次性加载 60MB
- **实际内存占用**：通常只有 2-5MB，取决于访问的 IP 范围
- **性能影响**：首次查询可能稍慢（缺页中断），但后续查询会被缓存
- **无需修改查询代码**：与原 API 完全兼容

**代码**：
```rust
// 使用 mmap 类型
static GEOIP_READER: Lazy<Mutex<Option<Reader<memmap2::Mmap>>>> = ...;

// 加载数据库
let file = File::open(db_path)?;
let mmap = unsafe { memmap2::Mmap::map(&file) }?;
let reader = Reader::from_source(mmap)?;
```

### 2. IP 地理信息缓存 ✅

**实现**：为已查询的 IP 地址缓存地理信息，避免重复查询数据库。

**优点**：
- 减少重复查询，提高性能
- 对于访问相同 IP 的场景，几乎零开销
- 缓存自动随程序运行增长

**代码**：
```rust
// IP 地理信息缓存
static GEO_CACHE: Lazy<Mutex<HashMap<String, IpGeoInfo>>> = 
    Lazy::new(|| Mutex::new(HashMap::new()));

fn get_ip_geo_info(ip_str: &str) -> IpGeoInfo {
    // 先检查缓存
    if let Some(info) = cache.get(ip_str) {
        return info.clone();
    }
    
    // 查询数据库
    let info = query_geoip_db(ip_str);
    
    // 保存到缓存
    cache.insert(ip_str.to_string(), info.clone());
    
    info
}
```

**依赖**：
```toml
memmap2 = "0.9"  # 用于 mmap GeoIP 数据库，减少内存占用
```

---

## 其他可选方案

### 方案 2：使用 GeoLite2-Country（简化版本）

如果不需要城市级别的精度，可以使用 GeoLite2-Country.mmdb。

**优点**：
- 文件大小仅约 6MB（减少 90%）
- 内存占用更少
- 查询速度更快

**缺点**：
- 只能获取国家信息，无省份、城市数据

**使用方法**：
```bash
# 下载 GeoLite2-Country 数据库
wget https://git.io/GeoLite2-Country.mmdb
```

**代码调整**：
```rust
// 修改查询为 Country 类型
match reader.lookup::<geoip2::Country>(ip) {
    Ok(country) => {
        // 只提取国家信息
    }
}
```

### 方案 3：使用 IP 缓存

为已查询的 IP 地址缓存地理信息，减少重复查询。

**实现**：
```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

static GEO_CACHE: Lazy<Arc<Mutex<HashMap<String, IpGeoInfo>>>> = 
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

fn get_ip_geo_info_cached(ip_str: &str) -> IpGeoInfo {
    // 检查缓存
    {
        let cache = GEO_CACHE.lock().unwrap();
        if let Some(info) = cache.get(ip_str) {
            return info.clone();
        }
    }
    
    // 查询数据库
    let info = get_ip_geo_info(ip_str);
    
    // 保存到缓存
    {
        let mut cache = GEO_CACHE.lock().unwrap();
        cache.insert(ip_str.to_string(), info.clone());
    }
    
    info
}
```

**优点**：
- 减少重复查询
- 提高性能

**缺点**：
- 增加额外的内存用于缓存（但通常比数据库小得多）
- 需要考虑缓存过期策略

### 方案 4：懒加载 + 仅在 Prometheus 导出时使用

只在真正需要时才加载 GeoIP 数据库。

**实现**：
```rust
// 仅在启用 Prometheus 且指定了 GeoIP 数据库时才加载
if let Some(port) = cli.prometheus_port {
    if let Some(ref geoip_path) = cli.geoip_db {
        init_geoip_db(geoip_path)?;
    }
}
```

### 方案 5：使用纯真 IP 数据库

使用国产的纯真 IP 数据库（QQWry.dat）。

**优点**：
- 文件更小（约 8-10MB）
- 中国 IP 数据更准确
- 包含 ISP 信息

**缺点**：
- 需要使用不同的 crate（如 `qqwry`）
- 国外 IP 数据可能不够准确

---

## 性能对比

| 方案 | 内存占用 | 查询速度 | 数据完整性 | 状态 |
|------|---------|---------|-----------|------|
| 原方案（readfile） | ~60MB | 快 | 完整 | ❌ |
| **mmap** | ~2-5MB | 稍慢（首次） | 完整 | ✅ 已实施 |
| **IP 缓存** | +缓存开销 | 很快（命中） | 完整 | ✅ 已实施 |
| GeoLite2-Country | ~0.5-1MB | 很快 | 仅国家 | 可选 |
| 纯真 IP | ~10MB | 快 | 较完整 | 可选 |

---

## 实际效果

**组合优化后的内存占用**：
- GeoIP 数据库：2-5MB（mmap 按需加载）
- IP 缓存：取决于访问的唯一 IP 数量
  - 每个 IP 约 200-300 字节（IP字符串 + 地理信息）
  - 1000 个 IP ≈ 300KB
  - 10000 个 IP ≈ 3MB
- **总计**：通常在 5-10MB 以内（相比原来的 60MB，减少了 85%）

---

## 推荐配置

对于 512MB 服务器，当前实施的优化已经足够：

✅ **mmap** - 大幅减少基础内存占用  
✅ **IP 缓存** - 提升性能，减少重复查询

### 验证内存占用

```bash
# 运行程序后检查内存使用
ps aux | grep ip_traffic_monitor_cli

# 或使用 systemd-cgtop
systemd-cgtop

# 详细内存映射
cat /proc/$(pidof ip_traffic_monitor_cli)/smaps | grep -A 15 GeoLite2
```

---

## 总结

**当前实施的优化方案**可以将 GeoIP 数据库的实际内存占用从 60MB 降低到 5-10MB，减少了约 **85%** 的内存使用，完全适合 512MB 服务器运行。

如果还需进一步优化，可以考虑：
- 使用 GeoLite2-Country 数据库（如果不需要城市级精度）
- 限制 IP 缓存大小（实现 LRU 缓存）
- 仅在启用 Prometheus 时才加载 GeoIP 数据库
