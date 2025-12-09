#!/bin/bash

export BPFTRACE_ON_STACK_LIMIT=128
cargo build --release
#     --backend bpftrace \
#    --geoip-db GeoLite2-City.mmdb \
sudo ./target/release/ip_traffic_monitor_cli \
    -i enp2s0 \
    --duration 0 \
    --sample-interval 2 \
    --prometheus-port 9091 \
    --backend bpftrace
    >ip_traffic_monitor.log 2>&1
