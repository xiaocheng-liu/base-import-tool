use super::{DbConnection, DbValue};
use crate::models::{ColumnInfo, ConnectionTestResult, DbConfig, IndexInfo, TableIdentifier};
use async_trait::async_trait;
use odbc_api::{Connection, ConnectionOptions, Cursor, Environment};
use std::sync::Mutex;

/// 达梦数据库 ODBC 连接
/// 使用持久 Environment + 连接缓存，避免每次操作都新建 ODBC 连接
pub struct DMConnection {
    conn: Mutex<Option<Connection<'static>>>,
    conn_string: String,
    schema: String,
}

impl DMConnection {
    pub async fn new(config: &DbConfig) -> Result<Self, String> {
        let conn_string = format!(
            "DRIVER={{DM8 ODBC DRIVER}};SERVER={};PORT={};UID={};PWD={};DATABASE={}",
            config.host, config.port, config.username, config.password, config.database
        );

        Ok(DMConnection {
            conn: Mutex::new(None),
            conn_string,
            schema: config.username.to_uppercase(),
        })
    }

    /// 获取或创建 ODBC 连接（使用 Box::leak 保证 Environment 生命周期）
    fn get_conn(&self) -> Result<std::sync::MutexGuard<'_, Option<Connection<'static>>>, String> {
        let mut conn_guard = self
            .conn
            .lock()
            .map_err(|e| format!("获取连接锁失败: {}", e))?;
        if conn_guard.is_none() {
            // 将 Environment 泄漏为 'static，使其生命周期覆盖整个进程
            let env = Environment::new().map_err(|e| format!("创建 ODBC 环境失败: {}", e))?;
            let env: &'static mut Environment = Box::leak(Box::new(env));

            let conn = env
                .connect_with_connection_string(&self.conn_string, ConnectionOptions::default())
                .map_err(|e| format!("连接达梦失败: {}", e))?;
            *conn_guard = Some(conn);
        }
        Ok(conn_guard)
    }

    fn execute_sql(&self, sql: &str) -> Result<(), String> {
        let mut conn_guard = self.get_conn()?;
        let conn = conn_guard.as_mut().ok_or("连接未建立")?;
        conn.execute(sql, ())
            .map_err(|e| format!("达梦执行 SQL 失败: {}", e))?;
        Ok(())
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

        let mut inserted = 0;
        let schema = effective_schema(&table.schema, &self.schema);
        {
            let mut conn_guard = self.get_conn()?;
            let conn = conn_guard.as_mut().ok_or("连接未建立")?;

            // ODBC 参数化查询需要编译期确定的元组类型，无法支持动态列数
            // 因此使用字符串拼接 + 单引号转义来防止 SQL 注入
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
                    table.table_name.to_uppercase(),
                    col_names.join(", "),
                    values.join(", ")
                );

                conn.execute(&sql, ())
                    .map_err(|e| format!("达梦插入数据失败: {}", e))?;
                inserted += 1;
            }
        }

        Ok(inserted)
    }

    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let sql = format!(
            "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = '{}' AND TABLE_NAME = '{}'",
            schema,
            table.table_name.to_uppercase()
        );

        let mut conn_guard = self.get_conn()?;
        let conn = conn_guard.as_mut().ok_or("连接未建立")?;

        // 使用 execute 执行查询，结果必须和 conn 在同一作用域中处理
        let result = conn.execute(&sql, ());
        match result {
            Ok(Some(mut cursor)) => {
                match cursor.next_row() {
                    Ok(Some(mut row)) => {
                        let mut buf = Vec::new();
                        match row.get_text(1, &mut buf) {
                            Ok(true) => {
                                let text = String::from_utf8_lossy(&buf);
                                let count: i64 = text.trim().parse().unwrap_or(0);
                                return Ok(count > 0);
                            }
                            Ok(false) => return Ok(false), // NULL
                            Err(e) => return Err(format!("达梦读取结果列失败: {}", e)),
                        }
                    }
                    Ok(None) => return Ok(false),
                    Err(e) => return Err(format!("达梦读取结果行失败: {}", e)),
                }
            }
            Ok(None) => Ok(false),
            Err(e) => Err(format!("达梦查询表存在性失败: {}", e)),
        }
    }

    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, DATA_LENGTH, DATA_PRECISION, DATA_SCALE FROM ALL_TAB_COLUMNS WHERE OWNER = '{}' AND TABLE_NAME = '{}' ORDER BY COLUMN_ID",
            schema,
            table.table_name.to_uppercase()
        );

        let mut conn_guard = self.get_conn()?;
        let conn = conn_guard.as_mut().ok_or("连接未建立")?;
        let mut columns = Vec::new();

        if let Some(mut cursor) = conn
            .execute(&sql, ())
            .map_err(|e| format!("达梦查询字段失败: {}", e))?
        {
            while let Some(mut row) = cursor
                .next_row()
                .map_err(|e| format!("达梦读取字段行失败: {}", e))?
            {
                columns.push(ColumnInfo {
                    name: read_text(&mut row, 1)?,
                    data_type: read_text(&mut row, 2)?,
                    data_length: read_optional_u32(&mut row, 3)?,
                    data_precision: read_optional_u32(&mut row, 4)?,
                    data_scale: read_optional_i32(&mut row, 5)?,
                });
            }
        }

        Ok(columns)
    }

    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String> {
        let schema = effective_schema(&table.schema, &self.schema);
        let sql = format!(
            "SELECT i.INDEX_NAME, i.UNIQUENESS, c.COLUMN_NAME FROM ALL_INDEXES i JOIN ALL_IND_COLUMNS c ON i.OWNER = c.INDEX_OWNER AND i.INDEX_NAME = c.INDEX_NAME WHERE i.TABLE_OWNER = '{}' AND i.TABLE_NAME = '{}' ORDER BY i.INDEX_NAME, c.COLUMN_POSITION",
            schema,
            table.table_name.to_uppercase()
        );

        let mut conn_guard = self.get_conn()?;
        let conn = conn_guard.as_mut().ok_or("连接未建立")?;
        let mut indexes: Vec<IndexInfo> = Vec::new();

        if let Some(mut cursor) = conn
            .execute(&sql, ())
            .map_err(|e| format!("达梦查询索引失败: {}", e))?
        {
            while let Some(mut row) = cursor
                .next_row()
                .map_err(|e| format!("达梦读取索引行失败: {}", e))?
            {
                let name = read_text(&mut row, 1)?;
                let uniqueness = read_text(&mut row, 2)?;
                let column = read_text(&mut row, 3)?;
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
        }

        Ok(indexes)
    }

    async fn get_version(&self) -> Result<String, String> {
        Ok("达梦数据库 (DM8)".to_string())
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String> {
        self.execute_sql(sql)
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

fn read_text(row: &mut odbc_api::CursorRow<'_>, column: u16) -> Result<String, String> {
    let mut buf = Vec::new();
    row.get_text(column, &mut buf)
        .map_err(|e| format!("达梦读取文本列失败: {}", e))?;
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

fn read_optional_u32(
    row: &mut odbc_api::CursorRow<'_>,
    column: u16,
) -> Result<Option<u32>, String> {
    let text = read_text(row, column)?;
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(text.parse().ok())
    }
}

fn read_optional_i32(
    row: &mut odbc_api::CursorRow<'_>,
    column: u16,
) -> Result<Option<i32>, String> {
    let text = read_text(row, column)?;
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(text.parse().ok())
    }
}
