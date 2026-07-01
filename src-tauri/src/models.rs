use serde::{Deserialize, Serialize};

/// 数据库类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DbType {
    Oracle,
    DM,
    PostgreSQL,
    MySQL,
}

impl std::fmt::Display for DbType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbType::Oracle => write!(f, "Oracle"),
            DbType::DM => write!(f, "DM"),
            DbType::PostgreSQL => write!(f, "PostgreSQL"),
            DbType::MySQL => write!(f, "MySQL"),
        }
    }
}

/// 目标数据库。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TargetDb {
    #[serde(rename = "cbs")]
    Cbs,
    #[serde(rename = "clin_wkst")]
    ClinWkst,
    #[serde(rename = "kbe")]
    #[default]
    Kbe,
    #[serde(rename = "drug_spec")]
    DrugSpec,
    #[serde(rename = "his")]
    His,
    #[serde(rename = "inpt")]
    Inpt,
    #[serde(rename = "kb_docs")]
    KbDocs,
    #[serde(rename = "outpt")]
    Outpt,
    #[serde(rename = "procure")]
    Procure,
}

impl std::fmt::Display for TargetDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetDb::Cbs => write!(f, "cbs"),
            TargetDb::ClinWkst => write!(f, "clin_wkst"),
            TargetDb::Kbe => write!(f, "kbe"),
            TargetDb::DrugSpec => write!(f, "drug_spec"),
            TargetDb::His => write!(f, "his"),
            TargetDb::Inpt => write!(f, "inpt"),
            TargetDb::KbDocs => write!(f, "kb_docs"),
            TargetDb::Outpt => write!(f, "outpt"),
            TargetDb::Procure => write!(f, "procure"),
        }
    }
}

impl TargetDb {
    /// 从数据库目录名判断目标数据库。
    pub fn from_dir_name(dir_name: &str) -> Option<Self> {
        match dir_name.to_lowercase().as_str() {
            "cbs" => Some(TargetDb::Cbs),
            "clin_wkst" => Some(TargetDb::ClinWkst),
            "kbe" => Some(TargetDb::Kbe),
            "drug_spec" => Some(TargetDb::DrugSpec),
            "his" => Some(TargetDb::His),
            "inpt" => Some(TargetDb::Inpt),
            "kb_docs" => Some(TargetDb::KbDocs),
            "outpt" => Some(TargetDb::Outpt),
            "procure" => Some(TargetDb::Procure),
            _ => None,
        }
    }
}

/// Schema 初始化脚本信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchemaTarget {
    pub target_db: String,
    pub tables_file: String,
    pub indexes_file: String,
}

/// 表字段及注释信息（用于前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnWithComment {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub comment: Option<String>,
}

/// 表结构信息（含字段和注释）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchemaInfo {
    pub table_name: String,
    pub table_comment: Option<String>,
    pub columns: Vec<ColumnWithComment>,
}

/// 表唯一标识。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableIdentifier {
    pub schema: String,
    pub table_name: String,
}

/// 数据库字段元数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub data_length: Option<u32>,
    pub data_precision: Option<u32>,
    pub data_scale: Option<i32>,
}

/// 数据库索引元数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// CSV 文件信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImportFileType {
    Csv,
    Sql,
}

/// 导入文件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvFileInfo {
    pub file_name: String,
    pub file_path: String,
    pub file_type: ImportFileType,
    pub target_db: String,
    pub table_name: String,
    pub row_count: Option<usize>,
    pub columns: Vec<String>,
}

/// Oracle 连接模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum OracleConnectionMode {
    #[default]
    ServiceName,
    SID,
}

/// 数据库连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConfig {
    pub id: String,
    pub db_type: DbType,
    #[serde(default)]
    pub target_db: TargetDb,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,     // PG的数据库名 / Oracle的ServiceName或SID / DM的schema
    #[serde(default)]
    pub oracle_connection_mode: OracleConnectionMode,
    pub extra_params: String, // 额外连接参数
}

/// 导入任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportTask {
    pub id: String,
    pub csv_file: CsvFileInfo,
    pub db_config_id: String,
    pub status: ImportStatus,
    pub progress: f64, // 0-100
    pub total_rows: usize,
    pub imported_rows: usize,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImportStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// 单条 SQL 执行失败记录（用于前端逐条展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlErrorItem {
    /// 第几条 SQL（从 1 开始）
    pub index: usize,
    /// 错误简述
    pub error: String,
    /// 完整的出错的 SQL 语句
    pub sql: String,
    /// 错误解决建议（可选）
    pub suggestion: Option<String>,
}

/// 导入进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportProgress {
    pub task_id: String,
    pub status: ImportStatus,
    pub progress: f64,
    pub total_rows: usize,
    pub imported_rows: usize,
    /// 失败 SQL 列表（按每条独立展示）
    #[serde(default)]
    pub errors: Vec<SqlErrorItem>,
    /// 兼容旧字段：单字符串错误信息（由 `errors` 汇总得到）
    #[serde(default)]
    pub error_message: Option<String>,
}

/// 测试连接结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestResult {
    pub success: bool,
    pub message: String,
    pub db_version: Option<String>,
}
