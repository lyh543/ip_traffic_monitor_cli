# Diesel 快速参考

## 🚀 快速开始

```bash
# 1. 安装 Diesel CLI
cargo install diesel_cli --no-default-features --features sqlite

# 2. 设置环境变量
echo "DATABASE_URL=ip_traffic_stats_orm.db" > .env

# 3. 初始化项目
diesel setup

# 4. 创建 migration
diesel migration generate create_table_name

# 5. 编写 SQL（手动编辑 up.sql 和 down.sql）

# 6. 运行 migration
diesel migration run

# 7. 生成 schema
diesel print-schema > src/schema.rs
```

## 📋 常用命令速查

| 命令 | 描述 |
|------|------|
| `diesel setup` | 初始化数据库和 migrations 目录 |
| `diesel migration generate <name>` | 生成新的 migration 文件 |
| `diesel migration run` | 运行所有未执行的 migrations |
| `diesel migration list` | 查看 migration 状态 |
| `diesel migration revert` | 回滚最后一个 migration |
| `diesel migration redo` | 重新运行所有 migrations |
| `diesel print-schema` | 生成 schema.rs |
| `diesel database reset` | 重置数据库 |

## 🔧 Cargo.toml 配置

```toml
[dependencies]
diesel = { version = "2.1", features = ["sqlite", "chrono"] }
diesel_migrations = "2.1"
chrono = { version = "0.4.38", features = ["serde"] }
```

## 📄 配置文件模板

### .env
```
DATABASE_URL=ip_traffic_stats_orm.db
```

### diesel.toml
```toml
[print_schema]
file = "src/schema.rs"
custom_type_derives = ["diesel::query_builder::QueryId", "Clone"]

[migrations_directory]
dir = "migrations"
```

## 🗃️ Migration SQL 模板

### up.sql
```sql
-- 创建表
CREATE TABLE table_name (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    column1 TEXT NOT NULL,
    column2 INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 创建索引
CREATE INDEX idx_table_name_column1 ON table_name(column1);
```

### down.sql
```sql
-- 删除索引
DROP INDEX IF EXISTS idx_table_name_column1;

-- 删除表
DROP TABLE table_name;
```

## 💡 实用代码片段

### 基本模型定义

```rust
use diesel::prelude::*;

#[derive(Queryable, Debug)]
pub struct MyModel {
    pub id: i32,
    pub name: String,
    pub created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = my_table)]
pub struct NewMyModel<'a> {
    pub name: &'a str,
}
```

### 数据库连接

```rust
use diesel::sqlite::SqliteConnection;
use diesel::Connection;
use std::env;

pub fn establish_connection() -> SqliteConnection {
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    SqliteConnection::establish(&database_url)
        .expect(&format!("Error connecting to {}", database_url))
}
```

### 基本 CRUD 操作

```rust
// 插入
diesel::insert_into(my_table::table)
    .values(&new_record)
    .execute(&mut connection)?;

// 查询
let results = my_table::table
    .load::<MyModel>(&mut connection)?;

// 更新
diesel::update(my_table::table.find(id))
    .set(my_table::name.eq("new_name"))
    .execute(&mut connection)?;

// 删除
diesel::delete(my_table::table.find(id))
    .execute(&mut connection)?;
```