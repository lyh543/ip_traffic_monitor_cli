# Diesel ORM 配置与使用指南

本文档详细介绍了如何在 Rust 项目中配置和使用 Diesel ORM 进行数据库操作。

## 📋 目录

- [环境要求](#环境要求)
- [Cargo 环境配置](#cargo-环境配置)
- [Diesel CLI 安装](#diesel-cli-安装)
- [项目初始化](#项目初始化)
- [Migration 管理](#migration-管理)
- [Schema 生成](#schema-生成)
- [常用命令](#常用命令)
- [故障排除](#故障排除)

## 🔧 环境要求

- Rust 1.86.0+ (推荐使用最新稳定版)
- SQLite 3.x (用于本项目)
- Git (版本控制)

### 检查当前环境

```bash
# 检查 Rust 版本
rustc --version
cargo --version

# 检查 SQLite 版本
sqlite3 --version
```

## 📦 Cargo 环境配置

### 1. 在 `Cargo.toml` 中添加依赖

```toml
[package]
name = "ip_traffic_monitor_cli"
version = "0.1.0"
edition = "2021"

[dependencies]
# Diesel ORM 核心依赖
diesel = { version = "2.1", features = ["sqlite", "chrono"] }
diesel_migrations = "2.1"

# 其他依赖
chrono = { version = "0.4.38", features = ["serde"] }
clap = { version = "4.4", features = ["derive"] }
byteorder = "1.5.0"
hex = "0.4.3"
procfs = "0.16.0"
```

### 2. 功能特性说明

- `sqlite`: 支持 SQLite 数据库
- `chrono`: 时间处理集成
- `serde`: 序列化支持

## ⚙️ Diesel CLI 安装

### 安装 Diesel CLI

```bash
# 仅安装 SQLite 支持的版本（推荐）
cargo install diesel_cli --no-default-features --features sqlite

# 或者安装支持所有数据库的版本
cargo install diesel_cli
```

### 验证安装

```bash
diesel --version
# 输出示例: diesel Version: 2.3.3 Supported Backends: sqlite
```

## 🚀 项目初始化

### 1. 创建环境配置文件

创建 `.env` 文件：

```bash
echo "DATABASE_URL=ip_traffic_stats_orm.db" > .env
```

### 2. 创建 Diesel 配置文件

创建 `diesel.toml` 文件：

```toml
# For documentation on how to configure this file,
# see https://diesel.rs/guides/configuring-diesel-cli

[print_schema]
file = "src/schema.rs"
custom_type_derives = ["diesel::query_builder::QueryId", "Clone"]

[migrations_directory]
dir = "migrations"
```

### 3. 初始化 Diesel 项目

```bash
# 初始化数据库和 migrations 目录
diesel setup
```

这个命令会：
- 创建数据库文件 (如果不存在)
- 创建 `migrations` 目录
- 创建初始的 schema 迁移表

## 📊 Migration 管理

### 工作流程概述

Diesel 遵循 "Migration-First" 原则：

```
编写 Migration SQL → 运行 Migration → 自动生成 schema.rs
```

### 1. 生成新的 Migration

```bash
# 生成新的 migration 文件
diesel migration generate create_table_name

# 示例：创建 ip_traffic 表
diesel migration generate create_ip_traffic_table
```

这会在 `migrations/` 目录下创建：
- `YYYY-MM-DD-HHMMSS-NNNN_migration_name/up.sql`
- `YYYY-MM-DD-HHMMSS-NNNN_migration_name/down.sql`

### 2. 编写 Migration SQL

#### up.sql (创建表)

```sql
-- 创建 ip_traffic 表
CREATE TABLE ip_traffic (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    timestamp TEXT NOT NULL,
    remote_ip TEXT NOT NULL,
    tx_rate INTEGER NOT NULL,
    pid INTEGER
);
```

#### down.sql (回滚操作)

```sql
-- 撤销 up.sql 中的操作
DROP TABLE ip_traffic;
```

### 3. 运行 Migration

```bash
# 运行所有未执行的 migrations
diesel migration run

# 查看 migration 状态
diesel migration list

# 回滚最后一个 migration
diesel migration revert

# 重新运行所有 migrations
diesel migration redo
```

## 🗂️ Schema 生成

### 自动生成 schema.rs

```bash
# 生成或更新 schema.rs
diesel print-schema > src/schema.rs
```

### 生成的 Schema 示例

```rust
// @generated automatically by Diesel CLI.

diesel::table! {
    ip_traffic (id) {
        id -> Integer,
        timestamp -> Text,
        remote_ip -> Text,
        tx_rate -> Integer,
        pid -> Nullable<Integer>,
    }
}
```

### 在代码中使用 Schema

```rust
// src/main.rs
use diesel::prelude::*;

mod schema;
use schema::ip_traffic;

// 定义数据结构
#[derive(Queryable)]
pub struct IpTraffic {
    pub id: i32,
    pub timestamp: String,
    pub remote_ip: String,
    pub tx_rate: i32,
    pub pid: Option<i32>,
}

#[derive(Insertable)]
#[diesel(table_name = ip_traffic)]
pub struct NewIpTraffic<'a> {
    pub timestamp: &'a str,
    pub remote_ip: &'a str,
    pub tx_rate: i32,
    pub pid: Option<i32>,
}
```

## 📋 常用命令

### Migration 相关

```bash
# 查看所有 migrations 状态
diesel migration list

# 运行 migrations
diesel migration run

# 回滚最后一个 migration
diesel migration revert

# 重新运行所有 migrations（先回滚后运行）
diesel migration redo

# 验证 migrations（不实际执行）
diesel migration run --dry-run
```

### Schema 相关

```bash
# 生成 schema.rs
diesel print-schema > src/schema.rs

# 查看特定表的 schema
diesel print-schema --table ip_traffic
```

### 数据库相关

```bash
# 重置数据库（删除并重新创建）
diesel database reset

# 仅创建数据库
diesel setup
```

## 🔍 故障排除

### 常见错误及解决方案

#### 1. Rust 版本不兼容

**错误信息**：
```
rustc 1.80.1 is not supported by the following packages:
diesel@2.3.3 requires rustc 1.86.0
```

**解决方法**：
```bash
# 更新 Rust 到最新稳定版
rustup update stable
```

#### 2. Diesel CLI 未安装

**错误信息**：
```
command not found: diesel
```

**解决方法**：
```bash
cargo install diesel_cli --no-default-features --features sqlite
```

#### 3. 环境变量未设置

**错误信息**：
```
DatabaseError(Connection(CouldntGetConnectionString))
```

**解决方法**：
```bash
# 创建 .env 文件
echo "DATABASE_URL=ip_traffic_stats_orm.db" > .env
```

#### 4. Schema 文件不存在

**错误信息**：
```
file not found for module `schema`
```

**解决方法**：
```bash
# 生成 schema 文件
diesel print-schema > src/schema.rs
```

#### 5. Migration 文件为空

这是正常现象！Diesel 不会自动填写 migration 内容，需要手动编写 SQL。

### 调试技巧

1. **查看数据库结构**：
   ```bash
   sqlite3 ip_traffic_stats_orm.db ".schema"
   ```

2. **查看表数据**：
   ```bash
   sqlite3 ip_traffic_stats_orm.db "SELECT * FROM ip_traffic LIMIT 5;"
   ```

3. **检查 migration 历史**：
   ```bash
   sqlite3 ip_traffic_stats_orm.db "SELECT * FROM __diesel_schema_migrations;"
   ```

## 📁 项目结构

建议的项目目录结构：

```
ip_traffic_monitor_cli/
├── Cargo.toml              # 项目配置和依赖
├── Cargo.lock              # 依赖锁定文件 (gitignore)
├── diesel.toml             # Diesel 配置
├── .env                    # 环境变量 (gitignore)
├── .gitignore              # Git 忽略规则
├── ip_traffic_stats_orm.db # SQLite 数据库 (gitignore)
├── docs/                   # 文档目录
│   └── diesel-setup-guide.md
├── migrations/             # Migration 文件
│   ├── .diesel_lock
│   ├── .keep
│   └── YYYY-MM-DD-HHMMSS-NNNN_migration_name/
│       ├── up.sql
│       └── down.sql
└── src/                    # 源代码
    ├── main.rs
    └── schema.rs           # 自动生成的数据库 schema
```

## 🚀 最佳实践

1. **Migration 命名**：使用描述性名称，如 `create_ip_traffic_table`、`add_index_to_timestamp`

2. **版本控制**：
   - 提交 migration 文件到 Git
   - 不要提交数据库文件和 `.env`

3. **团队协作**：
   - 确保所有团队成员使用相同的 Rust 版本
   - 定期运行 `diesel migration run` 同步数据库结构

4. **生产部署**：
   - 在部署前测试所有 migrations
   - 备份数据库后再运行 migrations
   - 考虑使用 `--dry-run` 验证 migrations

## 📚 参考资源

- [Diesel 官方文档](https://diesel.rs/)
- [Diesel CLI 指南](https://diesel.rs/guides/getting-started)
- [SQLite 数据类型](https://www.sqlite.org/datatype3.html)
- [Rust 官方文档](https://doc.rust-lang.org/)