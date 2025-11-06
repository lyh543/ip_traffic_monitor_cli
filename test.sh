#!/bin/bash

cargo build --release
sudo ./target/release/ip_traffic_monitor_cli -i enp2s0 --duration 0
