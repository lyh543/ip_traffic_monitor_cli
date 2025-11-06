-- 创建 ip_traffic 表
CREATE TABLE ip_traffic (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    timestamp TEXT NOT NULL,
    remote_ip TEXT NOT NULL,
    tx_rate INTEGER NOT NULL,
    pid INTEGER
);
