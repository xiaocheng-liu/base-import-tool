use super::{format_db_error, DbConnection, DbValue};
use crate::models::{ColumnInfo, ConnectionTestResult, DbConfig, IndexInfo, OracleConnectionMode, TableIdentifier};
use async_trait::async_trait;
use oracle::{Connection, InitParams};
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::timeout;

pub struct OracleConnection {
    conn: Connection,
    schema: String,
}

impl OracleConnection {
    pub async fn new(config: &DbConfig) -> Result<Self, String> {
        init_oracle_client()?;

        // Oracle 连接字符串格式:
        //   ServiceName: //host:port/service_name
        //   SID:         //host:port:sid  (注意用冒号分隔)
        let connect_string = match config.oracle_connection_mode {
            OracleConnectionMode::SID => {
                format!("//{}:{}/{}", config.host, config.port, config.database)
            }
            OracleConnectionMode::ServiceName => {
                format!("//{}:{}/{}", config.host, config.port, config.database)
            }
        };

        let conn = timeout(Duration::from_secs(10), tokio::task::spawn_blocking({
            let username = config.username.clone();
            let password = config.password.clone();
            move || {
                Connection::connect(
                    username.as_str(),
                    password.as_str(),
                    connect_string,
                )
            }
        }))
        .await
        .map_err(|_| "连接 Oracle 超时 (10s)".to_string())?
        .map_err(|e| format_db_error("Oracle 连接任务失败", e))?
        .map_err(|e| format_oracle_connect_error(&e.to_string()))?;

        Ok(OracleConnection {
            conn,
            schema: config.username.to_uppercase(),
        })
    }
}

/// 格式化 Oracle 连接错误。
pub fn format_oracle_connect_error(error: &str) -> String {
    if error.contains("ORA-28041") {
        format!(
            "连接 Oracle 失败: {}。该错误通常表示 Oracle 客户端与数据库认证协议不兼容，请检查数据库 sqlnet.ora 的 SQLNET.ALLOWED_LOGON_VERSION_* 设置，或改用与数据库版本匹配的 Oracle Instant Client。",
            error
        )
    } else {
        format!("连接 Oracle 失败: {}", error)
    }
}

/// 初始化 Oracle 客户端动态库目录。
fn init_oracle_client() -> Result<(), String> {
    if let Some(dir) = resolve_oracle_client_lib_dir(
        env::var("ORACLE_CLIENT_LIB_DIR").ok().as_deref(),
        env::var("ORACLE_HOME").ok().as_deref(),
    ) {
        InitParams::new()
            .oracle_client_lib_dir(dir)
            .map_err(|e| format_db_error("设置 Oracle 客户端目录失败", e))?
            .init()
            .map_err(|e| format_db_error("初始化 Oracle 客户端失败", e))?;
    }

    Ok(())
}

/// 根据平台返回 Oracle 客户端库文件名。
fn oracle_client_lib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "oci.dll"
    } else if cfg!(target_os = "linux") {
        "libclntsh.so"
    } else {
        "libclntsh.dylib"
    }
}

/// 解析 Oracle 客户端动态库目录。
pub fn resolve_oracle_client_lib_dir(
    oracle_client_lib_dir: Option<&str>,
    oracle_home: Option<&str>,
) -> Option<PathBuf> {
    let lib_name = oracle_client_lib_name();

    if let Some(dir) = oracle_client_lib_dir.map(Path::new) {
        if dir.join(lib_name).exists() {
            return Some(dir.to_path_buf());
        }
    }

    let home = Path::new(oracle_home?);
    if home.join(lib_name).exists() {
        Some(home.to_path_buf())
    } else if home.join("lib").join(lib_name).exists() {
        Some(home.join("lib"))
    } else {
        None
    }
}

#[async_trait]
impl DbConnection for OracleConnection {
    async fn test_connection(&self) -> Result<ConnectionTestResult, String> {
        let version = self.get_version().await?;
        Ok(ConnectionTestResult {
            success: true,
            message: "Oracle 连接成功".to_string(),
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
        let placeholders: Vec<String> = (0..columns.len()).map(|i| format!(":{}", i + 1)).collect();
        let schema = effective_schema(&table.schema, &self.schema);

        let sql = format!(
            "INSERT INTO \"{}\".\"{}\" ({}) VALUES ({})",
            schema,
            table.table_name.to_uppercase(),
            col_names.join(", "),
            placeholders.join(", ")
        );

        let mut inserted = 0;
        for row in &rows {
            // 每行重建 statement，避免上一行的绑定残留
            let mut stmt = self
                .conn
                .statement(&sql)
                .build()
                .map_err(|e| format_db_error("准备 Oracle 插入语句失败", e))?;

            for (i, val) in row.iter().enumerate() {
                match val {
                    DbValue::Null => stmt.bind(
                        i + 1,
                        &Option::<String>::None as &dyn oracle::sql_type::ToSql,
                    ),
                    DbValue::Text(value) => stmt.bind(i + 1, value as &dyn oracle::sql_type::ToSql),
                }
                .map_err(|e| format_db_error("Oracle 绑定参数失败", e))?;
            }
            stmt.execute(&[])
                .map_err(|e| format_db_error("Oracle 插入数据失败", e))?;
            inserted += 1;
        }

        // 提交事务
        if !self.conn.autocommit() {
            self.conn
                .commit()
                .map_err(|e| format_db_error("Oracle 提交失败", e))?;
        }

        Ok(inserted)
    }

    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String> {
        let sql = "SELECT COUNT(*) FROM ALL_TABLES WHERE OWNER = :1 AND TABLE_NAME = :2";
        let owner = effective_schema(&table.schema, &self.schema);
        let mut result_set = self
            .conn
            .query_as::<i32>(
                sql,
                &[&owner.as_str(), &table.table_name.to_uppercase().as_str()],
            )
            .map_err(|e| format_db_error("Oracle 查询表存在性失败", e))?;

        // ResultSet 实现了 Iterator
        if let Some(Ok(count)) = result_set.next() {
            Ok(count > 0)
        } else {
            Ok(false)
        }
    }

    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String> {
        let owner = effective_schema(&table.schema, &self.schema);
        let sql = "SELECT COLUMN_NAME, DATA_TYPE, DATA_LENGTH, DATA_PRECISION, DATA_SCALE FROM ALL_TAB_COLUMNS WHERE OWNER = :1 AND TABLE_NAME = :2 ORDER BY COLUMN_ID";
        let rows = self
            .conn
            .query_as::<(String, String, Option<u32>, Option<u32>, Option<i32>)>(
                sql,
                &[&owner.as_str(), &table.table_name.to_uppercase().as_str()],
            )
            .map_err(|e| format_db_error("Oracle 查询字段失败", e))?;

        rows.map(|row| {
            let (name, data_type, data_length, data_precision, data_scale) =
                row.map_err(|e| format_db_error("Oracle 读取字段失败", e))?;
            Ok(ColumnInfo {
                name,
                data_type,
                data_length,
                data_precision,
                data_scale,
            })
        })
        .collect()
    }

    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String> {
        let owner = effective_schema(&table.schema, &self.schema);
        let sql = "SELECT i.INDEX_NAME, i.UNIQUENESS, c.COLUMN_NAME FROM ALL_INDEXES i JOIN ALL_IND_COLUMNS c ON i.OWNER = c.INDEX_OWNER AND i.INDEX_NAME = c.INDEX_NAME WHERE i.TABLE_OWNER = :1 AND i.TABLE_NAME = :2 ORDER BY i.INDEX_NAME, c.COLUMN_POSITION";
        let rows = self
            .conn
            .query_as::<(String, String, String)>(
                sql,
                &[&owner.as_str(), &table.table_name.to_uppercase().as_str()],
            )
            .map_err(|e| format_db_error("Oracle 查询索引失败", e))?;

        let mut indexes: Vec<IndexInfo> = Vec::new();
        for row in rows {
            let (name, uniqueness, column) =
                row.map_err(|e| format_db_error("Oracle 读取索引失败", e))?;
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
    }

    async fn get_version(&self) -> Result<String, String> {
        let mut result_set = self
            .conn
            .query_as::<String>("SELECT BANNER FROM V$VERSION WHERE ROWNUM = 1", &[])
            .map_err(|e| format_db_error("获取 Oracle 版本失败", e))?;

        Ok(result_set
            .next()
            .and_then(|r| r.ok())
            .unwrap_or_else(|| "Unknown".to_string()))
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String> {
        self.conn
            .execute(sql, &[])
            .map_err(|e| format_db_error("Oracle 执行 SQL 失败", e))?;
        Ok(())
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

