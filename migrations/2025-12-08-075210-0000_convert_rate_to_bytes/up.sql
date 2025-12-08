-- 将 tx_rate 列重命名为 tx_bytes，并将现有数据乘以 2（因为之前存储的是2秒的速率）
ALTER TABLE ip_traffic RENAME COLUMN tx_rate TO tx_bytes;

-- 更新现有数据：将速率转换为字节数（速率 * 2秒）
UPDATE ip_traffic SET tx_bytes = tx_bytes * 2;
