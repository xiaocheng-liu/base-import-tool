use super::{format_db_error, DbConnection, DbValue};
use crate::models::{ColumnInfo, ConnectionTestResult, DbConfig, IndexInfo, TableIdentifier};
use async_trait::async_trait;
use dameng::{Client, ConnectOptions};
use std::time::Duration;

/// 达梦数据库原生驱动连接（纯 Rust 实现，无需 ODBC）
pub struct DMConnection {
    host: String,
    port: u16,
    username: String,
    password: String,
    schema: String,
    connect_timeout: Duration,
}

impl DMConnection {
    pub async fn new(config: &DbConfig) -> Result<Self, String> {
        Ok(DMConnection {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
            password: config.password.clone(),
            schema: config.username.to_uppercase(),
            connect_timeout: Duration::from_secs(10),
        })
    }

    /// 在 spawn_blocking 中创建连接并执行操作，操作完成后连接自动释放
    async fn with_client<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut Client) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let host = self.host.clone();
        let port = self.port;
        let username = self.username.clone();
        let password = self.password.clone();
        let schema = self.schema.clone();
        let timeout = self.connect_timeout;

        tokio::task::spawn_blocking(move || {
            let opts = ConnectOptions::new(&host, port, &username, &password)
                .schema(&schema)
                .connect_timeout(timeout);
            let mut client = Client::connect_with(&opts)
                .map_err(|e| format_db_error("连接达梦失败", e))?;
            f(&mut client)
        })
        .await
        .map_err(|e| format_db_error("达梦连接任务失败", e))?
    }
}

#[async_trait]
impl DbConnection for DMConnection {
    async fn test_connection(&self) -> Result<ConnectionTestResult, String> {
        let version = self.get_version().await?;
        Ok(ConnectionTestResult {
            success: true,
            message: "达梦数据库连接成功".to_string(),
            db_version: Some(version),
        })
    }

    async fn insert_rows(
        &self,
        table: &TableIdentifier,
        columns: &[String],
        rows: Vec<Vec<DbValue>>,
    ) -> Result<usize, String> {
        if rows.is_empty() {
            return Ok(0);
        }

        let col_names: Vec<String> = columns
            .iter()
            .map(|c| format!("\"{}\"", c.to_uppercase()))
            .collect();
        let schema = effective_schema(&table.schema, &self.schema);
        let table_name = table.table_name.to_uppercase();

        self.with_client(move |client| {
            let mut inserted = 0;
            for row in &rows {
                let values: Vec<String> = row
                    .iter()
                    .map(|v| match v {
                        DbValue::Null => "NULL".to_string(),
                        DbValue::Text(value) => format!("'{}'", value.replace("'", "''")),
                    })
                    .collect();

                let sql = format!(
                    "INSERT INTO \"{}\".\"{}\" ({}) VALUES ({})",
                    schema,
                    table_name,
                    col_names.join(", "),
                    values.join(", ")
                );

                client
                    .execute(&sql)
                    .map_err(|e| format_db_error("达梦插入数据失败", e))?;
                inserted += 1;
            }
            Ok(inserted)
        })
        .await
    }

    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let table_name = table.table_name.to_uppercase();
        let sql = format!(
            "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = '{}' AND TABLE_NAME = '{}'",
            schema, table_name
        );

        self.with_client(move |client| {
            let rs = client
                .query(&sql)
                .map_err(|e| format_db_error("达梦查询表存在性失败", e))?;
            for row in rs.iter() {
                let count: i64 = row.get(0).unwrap_or(0);
                return Ok(count > 0);
            }
            Ok(false)
        })
        .await
    }

    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let table_name = table.table_name.to_uppercase();
        let sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, DATA_LENGTH, DATA_PRECISION, DATA_SCALE \
             FROM ALL_TAB_COLUMNS \
             WHERE OWNER = '{}' AND TABLE_NAME = '{}' \
             ORDER BY COLUMN_ID",
            schema, table_name
        );

        self.with_client(move |client| {
            let rs = client
                .query(&sql)
                .map_err(|e| format_db_error("达梦查询字段失败", e))?;
            let mut columns = Vec::new();
            for row in rs.iter() {
                columns.push(ColumnInfo {
                    name: read_text(&row, 0),
                    data_type: read_text(&row, 1),
                    data_length: read_optional_u32(&row, 2),
                    data_precision: read_optional_u32(&row, 3),
                    data_scale: read_optional_i32(&row, 4),
                });
            }
            Ok(columns)
        })
        .await
    }

    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let table_name = table.table_name.to_uppercase();
        let sql = format!(
            "SELECT i.INDEX_NAME, i.UNIQUENESS, c.COLUMN_NAME \
             FROM ALL_INDEXES i \
             JOIN ALL_IND_COLUMNS c ON i.OWNER = c.INDEX_OWNER AND i.INDEX_NAME = c.INDEX_NAME \
             WHERE i.TABLE_OWNER = '{}' AND i.TABLE_NAME = '{}' \
             ORDER BY i.INDEX_NAME, c.COLUMN_POSITION",
            schema, table_name
        );

        self.with_client(move |client| {
            let rs = client
                .query(&sql)
                .map_err(|e| format_db_error("达梦查询索引失败", e))?;
            let mut indexes: Vec<IndexInfo> = Vec::new();
            for row in rs.iter() {
                let name = read_text(&row, 0);
                let uniqueness = read_text(&row, 1);
                let column = read_text(&row, 2);
                if let Some(existing) = indexes.iter_mut().find(|i| i.name == name) {
                    existing.columns.push(column);
                } else {
                    indexes.push(IndexInfo {
                        name,
                        columns: vec![column],
                        unique: uniqueness == "UNIQUE",
                    });
                }
            }
            Ok(indexes)
        })
        .await
    }

    async fn get_version(&self) -> Result<String, String> {
        Ok("达梦数据库 (DM8)".to_string())
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String> {
        let sql = sql.to_string();
        self.with_client(move |client| {
            client
                .execute(&sql)
                .map_err(|e| format_db_error("达梦执行 SQL 失败", e))?;
            Ok(())
        })
        .await
    }
}

/// 返回有效 schema，DDL 中为空时使用连接用户。
fn effective_schema(schema: &str, default_schema: &str) -> String {
    if schema.trim().is_empty() {
        default_schema.to_uppercase()
    } else {
        schema.to_uppercase()
    }
}

/// 从结果行中读取文本列（不会失败，NULL 返回空字符串）
fn read_text(row: &dameng::QueryRowRef, column: usize) -> String {
    row.get_str(column).unwrap_or("").trim().to_string()
}

/// 从结果行中读取可选的 u32 列
fn read_optional_u32(row: &dameng::QueryRowRef, column: usize) -> Option<u32> {
    let text = read_text(row, column);
    if text.is_empty() {
        None
    } else {
        text.parse().ok()
    }
}

/// 从结果行中读取可选的 i32 列
fn read_optional_i32(row: &dameng::QueryRowRef, column: usize) -> Option<i32> {
    let text = read_text(row, column);
    if text.is_empty() {
        None
    } else {
        text.parse().ok()
    }
}
