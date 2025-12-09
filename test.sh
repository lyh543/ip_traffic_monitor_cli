#!/bin/bash

cargo build --release
#     --backend bpftrace \
sudo ./target/release/ip_traffic_monitor_cli \
    -i enp2s0 \
    --duration 0 \
    --sample-interval 2 \
    --prometheus-port 9091 \
    --geoip-db GeoLite2-City.mmdb \
    --backend iftop
    >ip_traffic_monitor.log 2>&1
