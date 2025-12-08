-- 回滚：将数据除以 2，然后重命名列
UPDATE ip_traffic SET tx_bytes = tx_bytes / 2;

ALTER TABLE ip_traffic RENAME COLUMN tx_bytes TO tx_rate;
