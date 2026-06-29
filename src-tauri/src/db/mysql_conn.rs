use super::{DbConnection, DbValue};
use crate::models::{ColumnInfo, ConnectionTestResult, DbConfig, IndexInfo, TableIdentifier};
use async_trait::async_trait;
use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use sqlx::Row;
use std::collections::HashMap;

pub struct MySqlConnection {
    pool: MySqlPool,
}

impl MySqlConnection {
    pub async fn new(config: &DbConfig) -> Result<Self, String> {
        let url = if config.database.is_empty() {
            format!(
                "mysql://{}:{}@{}:{}",
                config.username, config.password, config.host, config.port
            )
        } else {
            format!(
                "mysql://{}:{}@{}:{}/{}",
                config.username, config.password, config.host, config.port, config.database
            )
        };

        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .map_err(|e| format!("连接 MySQL 失败: {}", e))?;

        // 测试连接
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .map_err(|e| format!("MySQL 测试查询失败: {}", e))?;

        Ok(MySqlConnection { pool })
    }

    fn database_name(table: &TableIdentifier) -> Option<String> {
        if table.schema.trim().is_empty() {
            None
        } else {
            Some(table.schema.to_lowercase())
        }
    }

    pub fn format_table_name(table: &TableIdentifier) -> String {
        if table.schema.trim().is_empty() {
            format!("`{}`", table.table_name.to_lowercase())
        } else {
            format!(
                "`{}`.`{}`",
                table.schema.to_lowercase(),
                table.table_name.to_lowercase()
            )
        }
    }
}

#[async_trait]
impl DbConnection for MySqlConnection {
    async fn test_connection(&self) -> Result<ConnectionTestResult, String> {
        let version = self.get_version().await?;
        Ok(ConnectionTestResult {
            success: true,
            message: "MySQL 连接成功".to_string(),
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
            .map(|c| format!("`{}`", c.to_lowercase()))
            .collect();
        let placeholders: Vec<String> = std::iter::repeat("?".to_string())
            .take(columns.len())
            .collect();

        // 生成 ON DUPLICATE KEY UPDATE 子句，用于处理主键冲突
        let update_assignments: Vec<String> = columns
            .iter()
            .map(|c| format!("`{}` = VALUES(`{}`)", c.to_lowercase(), c.to_lowercase()))
            .collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON DUPLICATE KEY UPDATE {}",
            Self::format_table_name(table),
            col_names.join(", "),
            placeholders.join(", "),
            update_assignments.join(", ")
        );

        let mut inserted = 0;
        for row in &rows {
            let mut query = sqlx::query(&sql);
            for val in row {
                query = match val {
                    DbValue::Null => query.bind(Option::<String>::None),
                    DbValue::Text(value) => query.bind(value),
                };
            }
            query
                .execute(&self.pool)
                .await
                .map_err(|e| format!("MySQL 插入数据失败: {}", e))?;
            inserted += 1;
        }

        Ok(inserted)
    }

    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM information_schema.tables WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ?",
        )
        .bind(Self::database_name(table))
        .bind(table.table_name.to_lowercase())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("MySQL 查询表存在性失败: {}", e))?;

        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }

    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String> {
        let rows = sqlx::query(
            "SELECT CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(DATA_TYPE AS CHAR) AS data_type, CAST(CHARACTER_MAXIMUM_LENGTH AS SIGNED) AS data_length, CAST(NUMERIC_PRECISION AS SIGNED) AS data_precision, CAST(NUMERIC_SCALE AS SIGNED) AS data_scale FROM information_schema.columns WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(Self::database_name(table))
        .bind(table.table_name.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("MySQL 查询字段失败: {}", e))?;

        rows.iter()
            .map(|row| {
                Ok(ColumnInfo {
                    name: row
                        .try_get::<String, _>("column_name")
                        .map_err(|e| format!("MySQL 读取字段名失败: {}", e))?,
                    data_type: row
                        .try_get::<String, _>("data_type")
                        .map_err(|e| format!("MySQL 读取字段类型失败: {}", e))?,
                    data_length: row
                        .try_get::<Option<i64>, _>("data_length")
                        .map_err(|e| format!("MySQL 读取字段长度失败: {}", e))?
                        .map(|v| v as u32),
                    data_precision: row
                        .try_get::<Option<i64>, _>("data_precision")
                        .map_err(|e| format!("MySQL 读取字段精度失败: {}", e))?
                        .map(|v| v as u32),
                    data_scale: row
                        .try_get::<Option<i64>, _>("data_scale")
                        .map_err(|e| format!("MySQL 读取字段小数位失败: {}", e))?
                        .map(|v| v as i32),
                })
            })
            .collect()
    }

    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String> {
        let rows = sqlx::query(
            "SELECT CAST(INDEX_NAME AS CHAR) AS index_name, NON_UNIQUE, CAST(COLUMN_NAME AS CHAR) AS column_name FROM information_schema.statistics WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? ORDER BY INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(Self::database_name(table))
        .bind(table.table_name.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("MySQL 查询索引失败: {}", e))?;

        let mut indexes: Vec<IndexInfo> = Vec::new();
        for row in rows {
            let name = row
                .try_get::<String, _>("index_name")
                .map_err(|e| format!("MySQL 读取索引名失败: {}", e))?;
            let non_unique = row
                .try_get::<i64, _>("NON_UNIQUE")
                .map_err(|e| format!("MySQL 读取索引唯一性失败: {}", e))?;
            let column = row
                .try_get::<String, _>("column_name")
                .map_err(|e| format!("MySQL 读取索引字段失败: {}", e))?;
            if let Some(existing) = indexes.iter_mut().find(|i| i.name == name) {
                existing.columns.push(column);
            } else {
                indexes.push(IndexInfo {
                    name,
                    columns: vec![column],
                    unique: non_unique == 0,
                });
            }
        }

        Ok(indexes)
    }

    async fn get_version(&self) -> Result<String, String> {
        let row = sqlx::query("SELECT VERSION() AS ver")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("获取 MySQL 版本失败: {}", e))?;

        Ok(row.get("ver"))
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String> {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("MySQL 执行 SQL 失败: {}", e))?;
        Ok(())
    }

    async fn get_table_comment(&self, table: &TableIdentifier) -> Result<Option<String>, String> {
        let row = sqlx::query(
            "SELECT CAST(TABLE_COMMENT AS CHAR) AS table_comment FROM information_schema.tables WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ?",
        )
        .bind(Self::database_name(table))
        .bind(table.table_name.to_lowercase())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("MySQL 查询表注释失败: {}", e))?;

        let comment: String = row.get("table_comment");
        if comment.is_empty() {
            Ok(None)
        } else {
            Ok(Some(comment))
        }
    }

    async fn get_column_comments(
        &self,
        table: &TableIdentifier,
    ) -> Result<HashMap<String, String>, String> {
        let rows = sqlx::query(
            "SELECT CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(COLUMN_COMMENT AS CHAR) AS column_comment FROM information_schema.columns WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND COLUMN_COMMENT != ''",
        )
        .bind(Self::database_name(table))
        .bind(table.table_name.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("MySQL 查询字段注释失败: {}", e))?;

        let mut comments = HashMap::new();
        for row in rows {
            let name: String = row.get("column_name");
            let comment: String = row.get("column_comment");
            comments.insert(name, comment);
        }
        Ok(comments)
    }
}

