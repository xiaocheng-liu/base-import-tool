pub mod dm_conn;
pub mod encoding;
pub mod mysql_conn;
pub mod oracle_conn;
pub mod pg_conn;

use crate::models::{
    ColumnInfo, ConnectionTestResult, DbConfig, DbType, IndexInfo, TableIdentifier,
};
use async_trait::async_trait;

/// 引入 encoding 工具，方便 db 子模块使用。
pub use encoding::{format_db_error, normalize_db_error};

/// 导入时使用的数据库值，CSV 空字段按 NULL 处理。
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Text(String),
}

/// 统一的数据库操作接口
#[async_trait]
pub trait DbConnection: Send + Sync {
    /// 测试连接
    async fn test_connection(&self) -> Result<ConnectionTestResult, String>;

    /// 批量插入数据
    async fn insert_rows(
        &self,
        table: &TableIdentifier,
        columns: &[String],
        rows: Vec<Vec<DbValue>>,
    ) -> Result<usize, String>;

    /// 检查带 schema 的表是否存在
    async fn schema_table_exists(&self, table: &TableIdentifier) -> Result<bool, String>;

    /// 获取表字段信息
    async fn get_columns(&self, table: &TableIdentifier) -> Result<Vec<ColumnInfo>, String>;

    /// 获取表索引信息
    async fn get_indexes(&self, table: &TableIdentifier) -> Result<Vec<IndexInfo>, String>;

    /// 获取数据库版本
    async fn get_version(&self) -> Result<String, String>;

    /// 执行原始 SQL（用于 DDL 操作）
    async fn execute_raw_sql(&self, sql: &str) -> Result<(), String>;

    /// 获取表注释，默认返回 None
    async fn get_table_comment(&self, _table: &TableIdentifier) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// 获取字段注释，返回 HashMap<列名, 注释>，默认返回空
    async fn get_column_comments(
        &self,
        _table: &TableIdentifier,
    ) -> Result<std::collections::HashMap<String, String>, String> {
        Ok(std::collections::HashMap::new())
    }
}

/// 根据配置创建数据库连接
pub async fn create_connection(config: &DbConfig) -> Result<Box<dyn DbConnection>, String> {
    match config.db_type {
        DbType::Oracle => {
            let conn = oracle_conn::OracleConnection::new(config).await?;
            Ok(Box::new(conn))
        }
        DbType::DM => {
            let conn = dm_conn::DMConnection::new(config).await?;
            Ok(Box::new(conn))
        }
        DbType::PostgreSQL => {
            let conn = pg_conn::PgConnection::new(config).await?;
            Ok(Box::new(conn))
        }
        DbType::MySQL => {
            let conn = mysql_conn::MySqlConnection::new(config).await?;
            Ok(Box::new(conn))
        }
    }
}
