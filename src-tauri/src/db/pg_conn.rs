use super::{DbConnection, DbValue};
use crate::models::{ColumnInfo, ConnectionTestResult, DbConfig, IndexInfo, TableIdentifier};
use async_trait::async_trait;
use deadpool_postgres::{Config as PoolConfig, Pool, Runtime};
use tokio_postgres::NoTls;

pub struct PgConnection {
    pool: Pool,
    schema: String,
}

impl PgConnection {
    pub async fn new(config: &DbConfig) -> Result<Self, String> {
        let mut cfg = PoolConfig::new();
        cfg.host = Some(config.host.clone());
        cfg.port = Some(config.port);
        cfg.user = Some(config.username.clone());
        cfg.password = Some(config.password.clone());
        cfg.dbname = Some(config.database.clone());

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("创建 PostgreSQL 连接池失败: {}", e))?;

        // 测试连接
        let client = pool
            .get()
            .await
            .map_err(|e| format!("连接 PostgreSQL 失败: {}", e))?;
        client
            .simple_query("SELECT 1")
            .await
            .map_err(|e| format!("PostgreSQL 测试查询失败: {}", e))?;

        Ok(PgConnection {
            pool,
            schema: config.username.to_lowercase(),
        })
    }
}

#[async_trait]
impl DbConnection for PgConnection {
    async fn test_connection(&self) -> Result<ConnectionTestResult, String> {
        let version = self.get_version().await?;
        Ok(ConnectionTestResult {
            success: true,
            message: "PostgreSQL 连接成功".to_string(),
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

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;

        let col_names: Vec<String> = columns
            .iter()
            .map(|c| format!("\"{}\"", c.to_lowercase()))
            .collect();
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${}", i)).collect();
        let schema = effective_schema(&table.schema, &self.schema);

        let sql = format!(
            "INSERT INTO \"{}\".\"{}\" ({}) VALUES ({})",
            schema,
            table.table_name.to_lowercase(),
            col_names.join(", "),
            placeholders.join(", ")
        );

        let stmt = client
            .prepare(&sql)
            .await
            .map_err(|e| format!("准备 PG 插入语句失败: {}", e))?;

        let mut inserted = 0;
        for row in &rows {
            let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = row
                .iter()
                .map(|value| match value {
                    DbValue::Null => &None::<String> as &(dyn tokio_postgres::types::ToSql + Sync),
                    DbValue::Text(text) => text as &(dyn tokio_postgres::types::ToSql + Sync),
                })
                .collect();

            client
                .execute(&stmt, &params)
                .await
                .map_err(|e| format!("PG 插入数据失败: {}", e))?;
            inserted += 1;
        }

        Ok(inserted)
    }

    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;
        let schema = effective_schema(&table.schema, &self.schema);
        let rows = client
            .query(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2",
                &[&schema, &table.table_name.to_lowercase()],
            )
            .await
            .map_err(|e| format!("PG 查询表存在性失败: {}", e))?;

        let count: i64 = rows.first().map(|r| r.get(0)).unwrap_or(0);
        Ok(count > 0)
    }

    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;
        let schema = effective_schema(&table.schema, &self.schema);
        let rows = client
            .query(
                "SELECT column_name, data_type, character_maximum_length, numeric_precision, numeric_scale FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                &[&schema, &table.table_name.to_lowercase()],
            )
            .await
            .map_err(|e| format!("PG 查询字段失败: {}", e))?;

        Ok(rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get::<_, String>(0),
                data_type: row.get::<_, String>(1),
                data_length: row.get::<_, Option<i32>>(2).map(|v| v as u32),
                data_precision: row.get::<_, Option<i32>>(3).map(|v| v as u32),
                data_scale: row.get::<_, Option<i32>>(4),
            })
            .collect())
    }

    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;
        let schema = effective_schema(&table.schema, &self.schema);
        let rows = client
            .query(
                "SELECT i.relname, ix.indisunique, a.attname FROM pg_class t JOIN pg_namespace n ON n.oid = t.relnamespace JOIN pg_index ix ON ix.indrelid = t.oid JOIN pg_class i ON i.oid = ix.indexrelid JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) WHERE n.nspname = $1 AND t.relname = $2 ORDER BY i.relname, array_position(ix.indkey, a.attnum)",
                &[&schema, &table.table_name.to_lowercase()],
            )
            .await
            .map_err(|e| format!("PG 查询索引失败: {}", e))?;

        let mut indexes: Vec<IndexInfo> = Vec::new();
        for row in rows {
            let name: String = row.get(0);
            let unique: bool = row.get(1);
            let column: String = row.get(2);
            if let Some(existing) = indexes.iter_mut().find(|i| i.name == name) {
                existing.columns.push(column);
            } else {
                indexes.push(IndexInfo {
                    name,
                    columns: vec![column],
                    unique,
                });
            }
        }
        Ok(indexes)
    }

    async fn get_version(&self) -> Result<String, String> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;
        let rows = client
            .query("SELECT version()", &[])
            .await
            .map_err(|e| format!("获取 PG 版本失败: {}", e))?;

        Ok(rows
            .first()
            .map(|r| r.get::<_, String>(0))
            .unwrap_or_else(|| "Unknown".to_string()))
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| format!("获取 PG 连接失败: {}", e))?;
        client
            .batch_execute(sql)
            .await
            .map_err(|e| format!("PostgreSQL 执行 SQL 失败: {}", e))?;
        Ok(())
    }
}

/// 返回有效 schema，DDL 中为空时使用 public。
fn effective_schema(schema: &str, _default_schema: &str) -> String {
    if schema.trim().is_empty() {
        "public".to_string()
    } else {
        schema.to_lowercase()
    }
}
