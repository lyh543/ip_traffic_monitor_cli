#!/bin/bash

set -e  # 遇到错误立即退出

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 远端配置
REMOTE_HOST="lyh543@frps.lyh543.cn"
REMOTE_PORT="22222"
REMOTE_DIR="/home/lyh543/workspace/ip_traffic_monitor_cli"
GEOIP_DB="GeoLite2-City.mmdb"
GEOIP_URL="https://git.io/GeoLite2-City.mmdb"

echo -e "${GREEN}开始部署 IP Traffic Monitor CLI...${NC}"

# 0. 下载 GeoIP 数据库（如果不存在）
echo -e "${YELLOW}步骤 0: 检查 GeoIP 数据库...${NC}"
if [[ ! -f "./${GEOIP_DB}" ]]; then
    echo "本地不存在 ${GEOIP_DB}，正在下载..."
    curl -L -o "${GEOIP_DB}" "${GEOIP_URL}"
    
    # 检查文件是否下载成功
    if [[ ! -f "./${GEOIP_DB}" ]]; then
        echo -e "${RED}错误: GeoIP 数据库下载失败${NC}"
        exit 1
    fi
    
    # 检查文件大小，确保下载完整
    FILE_SIZE=$(stat -f%z "${GEOIP_DB}" 2>/dev/null || stat -c%s "${GEOIP_DB}" 2>/dev/null)
    if [[ $FILE_SIZE -lt 1000000 ]]; then
        echo -e "${RED}错误: 下载的文件过小 (${FILE_SIZE} bytes)，可能不完整${NC}"
        rm -f "${GEOIP_DB}"
        exit 1
    fi
    
    echo -e "${GREEN}✓ GeoIP 数据库下载成功 (${FILE_SIZE} bytes)${NC}"
else
    echo -e "${GREEN}✓ 本地已存在 ${GEOIP_DB}${NC}"
fi

# 1. 构建项目
echo -e "${YELLOW}步骤 1: 构建项目...${NC}"
cargo build --release

# 2. 检查必要文件是否存在
if [[ ! -f "./target/release/ip_traffic_monitor_cli" ]]; then
    echo -e "${RED}错误: 构建失败，可执行文件不存在${NC}"
    exit 1
fi

# 3. 创建远端目录
echo -e "${YELLOW}步骤 2: 创建远端目录...${NC}"
ssh -p ${REMOTE_PORT} ${REMOTE_HOST} "mkdir -p ${REMOTE_DIR}"

# 4. 上传可执行文件
echo -e "${YELLOW}步骤 3: 上传可执行文件...${NC}"
rsync -avz -e "ssh -p ${REMOTE_PORT}" ./target/release/ip_traffic_monitor_cli ${REMOTE_HOST}:${REMOTE_DIR}/ip_traffic_monitor_cli

# 5. 上传 GeoIP 数据库
echo -e "${YELLOW}步骤 4: 上传 GeoIP 数据库...${NC}"
rsync -avz -e "ssh -p ${REMOTE_PORT}" "./${GEOIP_DB}" "${REMOTE_HOST}:${REMOTE_DIR}/${GEOIP_DB}"
echo -e "${GREEN}✓ GeoIP 数据库已上传${NC}"

# 6. 停止旧进程并启动新进程
echo -e "${YELLOW}步骤 5: 重启服务...${NC}"
ssh -p ${REMOTE_PORT} ${REMOTE_HOST} "
    echo '停止旧进程...'
    sudo killall ip_traffic_monitor_cli || true
    cd ${REMOTE_DIR}
    echo '使用 nohup 启动新进程（含 GeoIP 地理位置信息）...'
    nohup sudo ./ip_traffic_monitor_cli \
        --iface eth0 \
        --duration 0 \
        --sample-interval 10 \
        --prometheus-port 9091 \
        --geoip-db ${GEOIP_DB} \
        > ip_traffic_monitor.log 2>&1 &
    echo '进程已在后台启动，日志输出到 ip_traffic_monitor.log'
    echo 'Prometheus exporter 监听端口: 9091'
    sleep 2
    echo '检查进程状态:'
    if pgrep -f ip_traffic_monitor_cli > /dev/null; then
        echo '✓ 进程启动成功'
        echo '访问 http://服务器IP:9091/metrics 查看 Prometheus 指标'
    else
        echo '✗ 进程启动失败，请检查日志'
        tail -10 ip_traffic_monitor.log
    fi
"

echo -e "${GREEN}部署完成！${NC}"
echo -e "${GREEN}提示:${NC}"
echo -e "  - Prometheus metrics: http://${REMOTE_HOST}:9091/metrics"
echo -e "  - 查看日志: ssh -p ${REMOTE_PORT} ${REMOTE_HOST} 'tail -f ${REMOTE_DIR}/ip_traffic_monitor.log'"
echo -e "  - GeoIP 数据库: ${GREEN}已启用${NC} (包含国家/省份/城市信息)"
echo -e "  - 存储模式: ${GREEN}内存存储${NC} (无需数据库)"