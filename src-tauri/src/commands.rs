use crate::config_store;
use crate::csv_parser;
use crate::db::{self, DbConnection, DbValue};
use crate::ddl_converter::DdlConverter;
use crate::models::*;
use csv::ReaderBuilder;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;

/// 全局状态：存储数据库配置
pub struct AppState {
    pub db_configs: Mutex<Vec<DbConfig>>,
    pub db_config_path: PathBuf,
    pub import_progress: Arc<Mutex<HashMap<String, ImportProgress>>>,
}

/// 扫描文件夹，返回所有 CSV 文件信息
#[tauri::command]
pub fn scan_csv_files(folder_path: String) -> Result<Vec<CsvFileInfo>, String> {
    csv_parser::scan_folder(&folder_path)
}

/// 获取所有数据库配置
#[tauri::command]
pub fn get_db_configs(state: State<AppState>) -> Result<Vec<DbConfig>, String> {
    let configs = state.db_configs.lock().map_err(|e| e.to_string())?;
    Ok(configs.clone())
}

/// 保存数据库配置
#[tauri::command]
pub fn save_db_config(state: State<AppState>, config: DbConfig) -> Result<DbConfig, String> {
    let mut configs = state.db_configs.lock().map_err(|e| e.to_string())?;

    // 如果 ID 为空则生成新 ID
    let mut config = config;
    if config.id.is_empty() {
        config.id = Uuid::new_v4().to_string();
    }

    // 更新或添加
    if let Some(existing) = configs.iter_mut().find(|c| c.id == config.id) {
        *existing = config.clone();
    } else {
        configs.push(config.clone());
    }

    config_store::save_db_configs(&state.db_config_path, &configs)?;

    Ok(config)
}

/// 删除数据库配置
#[tauri::command]
pub fn delete_db_config(state: State<AppState>, id: String) -> Result<(), String> {
    let mut configs = state.db_configs.lock().map_err(|e| e.to_string())?;
    configs.retain(|c| c.id != id);
    config_store::save_db_configs(&state.db_config_path, &configs)
}

/// 测试数据库连接
#[tauri::command]
pub async fn test_connection(config: DbConfig) -> Result<ConnectionTestResult, String> {
    let conn = db::create_connection(&config).await?;
    conn.test_connection().await
}

/// 开始导入任务
#[tauri::command]
pub async fn start_import(
    state: State<'_, AppState>,
    csv_files: Vec<CsvFileInfo>,
    db_config_id: String,
) -> Result<Vec<ImportTask>, String> {
    let db_config = {
        let configs = state.db_configs.lock().map_err(|e| e.to_string())?;
        let cfg = configs
            .iter()
            .find(|c| c.id == db_config_id)
            .cloned()
            .ok_or_else(|| "未找到数据库配置".to_string())?;
        cfg
    }; // configs MutexGuard 在这里 drop

    let db_type = db_config.db_type.clone();
    let conn = db::create_connection(&db_config).await?;

    let mut tasks = Vec::new();

    for csv_file in &csv_files {
        let task_id = Uuid::new_v4().to_string();

        let total_rows = csv_file.row_count.unwrap_or(0);
        let task = ImportTask {
            id: task_id.clone(),
            csv_file: csv_file.clone(),
            db_config_id: db_config_id.clone(),
            status: ImportStatus::Pending,
            progress: 0.0,
            total_rows,
            imported_rows: 0,
            error_message: None,
        };

        // 更新进度
        {
            let mut progress_map = state.import_progress.lock().map_err(|e| e.to_string())?;
            progress_map.insert(
                task_id.clone(),
                ImportProgress {
                    task_id: task_id.clone(),
                    status: ImportStatus::Pending,
                    progress: 0.0,
                    total_rows,
                    imported_rows: 0,
                    error_message: None,
                    errors: vec![],
                },
            );
        }

        tasks.push(task);
    }

    // 异步执行导入，conn 必须 move 进去避免 use-after-free
    let progress_map = Arc::clone(&state.import_progress);
    let tasks_for_spawn = tasks.clone();
    tokio::spawn(async move {
        for task in &tasks_for_spawn {
            execute_import(&*conn, task, &progress_map, &db_type).await;
        }
        // conn 在这里 drop，确保所有任务完成后才释放连接
    });

    Ok(tasks)
}

/// 执行单个文件的导入
async fn execute_import(
    conn: &dyn DbConnection,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    db_type: &DbType,
) {
    // 更新状态为运行中
    update_progress(
        progress_map,
        &task.id,
        ImportStatus::Running,
        0.0,
        0,
        0,
        None,
    );

    match task.csv_file.file_type {
        ImportFileType::Csv => execute_csv_import(conn, task, progress_map).await,
        ImportFileType::Sql => execute_sql_import(conn, task, progress_map, db_type).await,
    }
}

/// 根据 CSV 所属数据库目录生成导入目标表。
pub fn import_target_table(csv_file: &CsvFileInfo) -> TableIdentifier {
    TableIdentifier {
        schema: csv_file.target_db.clone(),
        table_name: csv_file.table_name.clone(),
    }
}

/// 获取导入进度
#[tauri::command]
pub fn get_import_progress(
    state: State<AppState>,
    task_ids: Vec<String>,
) -> Result<HashMap<String, ImportProgress>, String> {
    let progress_map = state.import_progress.lock().map_err(|e| e.to_string())?;

    if task_ids.is_empty() {
        Ok(progress_map.clone())
    } else {
        let filtered: HashMap<String, ImportProgress> = task_ids
            .iter()
            .filter_map(|id| progress_map.get(id).map(|p| (id.clone(), p.clone())))
            .collect();
        Ok(filtered)
    }
}

/// 读取 CSV 文件的所有数据
pub struct CsvData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<DbValue>>,
}

async fn execute_csv_import(
    conn: &dyn DbConnection,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
) {
    let csv_data = match read_csv_data(&task.csv_file.file_path) {
        Ok(data) => data,
        Err(e) => {
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                0,
                0,
                Some(e),
            );
            return;
        }
    };

    let columns = &csv_data.columns;
    let rows = &csv_data.rows;
    let total_rows = rows.len();
    let target_table = import_target_table(&task.csv_file);
    let target_table_label = format!("{}.{}", target_table.schema, target_table.table_name);

    match conn.schema_table_exists(&target_table).await {
        Ok(true) => {}
        Ok(false) => {
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                0,
                0,
                Some(format!(
                    "目标表 {} 不存在，请先执行数据库初始化",
                    target_table_label
                )),
            );
            return;
        }
        Err(e) => {
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                0,
                0,
                Some(format!("检查目标表失败: {}", e)),
            );
            return;
        }
    }

    let batch_size = 500;
    let mut imported = 0usize;

    for chunk in rows.chunks(batch_size) {
        match conn
            .insert_rows(&target_table, columns, chunk.to_vec())
            .await
        {
            Ok(count) => {
                imported += count;
                let progress = if total_rows > 0 {
                    (imported as f64 / total_rows as f64) * 100.0
                } else {
                    100.0
                };
                update_progress(
                    progress_map,
                    &task.id,
                    ImportStatus::Running,
                    progress,
                    total_rows,
                    imported,
                    None,
                );
            }
            Err(e) => {
                update_progress(
                    progress_map,
                    &task.id,
                    ImportStatus::Failed,
                    0.0,
                    total_rows,
                    imported,
                    Some(format!("插入数据失败: {}", e)),
                );
                return;
            }
        }
    }

    update_progress(
        progress_map,
        &task.id,
        ImportStatus::Completed,
        100.0,
        total_rows,
        imported,
        None,
    );
}

async fn execute_sql_import(
    conn: &dyn DbConnection,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    db_type: &DbType,
) {
    let sql = match read_sql_data(&task.csv_file.file_path) {
        Ok(sql) => sql,
        Err(e) => {
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                0,
                0,
                Some(e),
            );
            return;
        }
    };

    let statements = prepare_sql_statements(&sql, db_type, &task.csv_file.target_db);
    if statements.is_empty() {
        update_progress(
            progress_map,
            &task.id,
            ImportStatus::Failed,
            0.0,
            0,
            0,
            Some("SQL 文件内容为空或无法识别有效语句".to_string()),
        );
        return;
    }

    let total = statements.len();
    for (index, statement) in statements.iter().enumerate() {
        if let Err(e) = conn.execute_raw_sql(statement).await {
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                total,
                index,
                Some(format!("执行 SQL 文件失败: {}", e)),
            );
            return;
        }

        let imported = index + 1;
        let progress = (imported as f64 / total as f64) * 100.0;
        let status = if imported == total {
            ImportStatus::Completed
        } else {
            ImportStatus::Running
        };
        update_progress(
            progress_map,
            &task.id,
            status,
            progress,
            total,
            imported,
            None,
        );
    }
}

pub fn prepare_sql_statements(sql: &str, db_type: &DbType, target_db: &str) -> Vec<String> {
    split_sql_statements(sql)
        .into_iter()
        .map(|stmt| adapt_sql_statement(&stmt, db_type, target_db))
        .filter(|stmt| !stmt.trim().is_empty())
        .collect()
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut previous = '\0';
    let mut in_line_comment = false;

    for ch in sql.chars() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                current.clear();
            }
            continue;
        }
        if ch == '-' && previous == '-' && !in_single_quote && !in_double_quote {
            // 进入行注释，回退已添加的 '-' 并清空当前行
            current.pop();
            in_line_comment = true;
            continue;
        }
        match ch {
            '\'' if previous != '\\' && !in_double_quote => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if previous != '\\' && !in_single_quote => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            ';' if !in_single_quote && !in_double_quote => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    result.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
        previous = ch;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        result.push(trimmed.to_string());
    }

    result
}

/// 将 Oracle 的 TO_DATE 函数转换为目标数据库的对应函数。
/// MySQL: `TO_DATE('2025-01-01', 'YYYY-MM-DD')` → `STR_TO_DATE('2025-01-01', '%Y-%m-%d')`
/// PostgreSQL: `TO_DATE(...)` → `TO_TIMESTAMP(...)`
fn convert_oracle_to_date(sql: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::MySQL => convert_to_date_inner(sql, "STR_TO_DATE", true),
        DbType::PostgreSQL => convert_to_date_inner(sql, "TO_TIMESTAMP", false),
        _ => sql.to_string(),
    }
}

/// 核心替换逻辑：找到所有 TO_DATE(...) 调用并替换为目标函数。
/// 注意排除已转换的 STR_TO_DATE 中包含的 TO_ 前缀。
fn convert_to_date_inner(sql: &str, target_func: &str, convert_format: bool) -> String {
    let upper = sql.to_uppercase();
    let mut result = String::with_capacity(sql.len());
    let mut pos = 0;

    loop {
        match find_sql_keyword(&upper[pos..], "TO_DATE") {
            Some(rel_pos) => {
                let abs_pos = pos + rel_pos;
                // 推入前面的内容
                result.push_str(&sql[pos..abs_pos]);
                // 找到 TO_DATE( 的括号范围
                let open = abs_pos + "TO_DATE".len();
                // 跳过 (
                let paren_start = open;
                if paren_start >= sql.len() || sql.as_bytes()[paren_start] != b'(' {
                    // 不应该走到这里，容错
                    result.push_str("TO_DATE");
                    pos = open;
                    continue;
                }
                // 找到匹配的 )
                let close = find_matching_paren(sql, paren_start);
                // 提取 TO_DATE 括号内的内容
                let inner = &sql[paren_start + 1..close];
                if convert_format {
                    let converted = convert_oracle_date_format(inner);
                    result.push_str(target_func);
                    result.push('(');
                    result.push_str(&converted);
                    result.push(')');
                } else {
                    result.push_str(target_func);
                    result.push('(');
                    result.push_str(inner);
                    result.push(')');
                }
                pos = close + 1;
            }
            None => {
                result.push_str(&sql[pos..]);
                break;
            }
        }
    }
    result
}

/// 找到与 pos 处 ( 匹配的 ) 位置。
fn find_matching_paren(s: &str, open_pos: usize) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 0;
    let mut in_single_quote = false;
    let mut prev = 0u8;

    for (i, &ch) in bytes[open_pos..].iter().enumerate() {
        if ch == b'\'' && prev != b'\\' {
            in_single_quote = !in_single_quote;
        } else if !in_single_quote {
            if ch == b'(' {
                depth += 1;
            } else if ch == b')' {
                depth -= 1;
                if depth == 0 {
                    return open_pos + i;
                }
            }
        }
        prev = ch;
    }
    s.len() - 1
}

/// 将 Oracle 日期格式字符串转换为 MySQL STR_TO_DATE 格式。
/// YYYY→%Y, MM→%m, DD→%d, HH24→%H, MI→%i, SS→%s
fn convert_oracle_date_format(inner: &str) -> String {
    // inner 格式: 'date_value', 'Oracle_format_string'
    // 找到第二个单引号字符串的位置，对其中的格式标识符做替换
    let bytes = inner.as_bytes();
    let mut in_quote = false;
    let mut escaped = false;
    let mut quote_count = 0;
    let mut format_start = 0;

    for (i, &ch) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == b'\\' {
            escaped = true;
            continue;
        }
        if ch == b'\'' {
            in_quote = !in_quote;
            if !in_quote {
                quote_count += 1;
            } else if quote_count == 1 {
                // 进入第二个引号字符串（格式字符串）
                format_start = i + 1;
            }
        }
    }

    let format_str = &inner[format_start..];
    // 提取引号内的格式字符串并做替换
    let format_inner = format_str.trim_end_matches('\'');
    let converted_format = format_inner
        .replace("HH24", "%H")
        .replace("YYYY", "%Y")
        .replace("MM", "%m")
        .replace("DD", "%d")
        .replace("MI", "%i")
        .replace("SS", "%s");

    format!("{}'{}'", &inner[..format_start], converted_format)
}

fn adapt_sql_statement(sql: &str, db_type: &DbType, target_db: &str) -> String {
    let with_date_conv = convert_oracle_to_date(sql, db_type);
    match db_type {
        DbType::MySQL => with_mysql_duplicate_key_update(&ensure_mysql_target_db(
            &replace_quoted_identifiers(&with_date_conv, '`', true),
            target_db,
        )),
        DbType::PostgreSQL => replace_quoted_identifiers(&with_date_conv, '"', true),
        _ => with_date_conv.trim().to_string(),
    }
}

fn replace_quoted_identifiers(sql: &str, quote_char: char, lowercase: bool) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_single_quote = false;

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            in_single_quote = !in_single_quote;
            result.push(ch);
            continue;
        }

        if ch == '"' && !in_single_quote {
            let mut identifier = String::new();
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == '"' {
                    break;
                }
                identifier.push(next);
            }

            let identifier = if lowercase {
                identifier.to_lowercase()
            } else {
                identifier
            };
            result.push(quote_char);
            result.push_str(&identifier);
            result.push(quote_char);
            continue;
        }

        result.push(ch);
    }

    result.trim().to_string()
}

fn with_mysql_duplicate_key_update(sql: &str) -> String {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    if !upper.starts_with("INSERT INTO ") || upper.contains(" ON DUPLICATE KEY UPDATE ") {
        return trimmed.to_string();
    }

    let Some(columns_start) = trimmed.find('(') else {
        return trimmed.to_string();
    };
    // 查找 VALUES 或 SELECT 关键字位置，兼容紧凑格式如 )VALUES(
    let insert_head_end = if let Some(pos) = upper.find(" VALUES ") {
        pos
    } else if let Some(pos) = find_sql_keyword(&upper, "VALUES") {
        // find_sql_keyword 返回关键字起始位置；列段应以关键字前的 ) 结尾
        upper[..pos].rfind(')').unwrap_or(pos)
    } else if let Some(pos) = upper.find(" SELECT ") {
        pos
    } else if let Some(pos) = find_sql_keyword(&upper, "SELECT") {
        upper[..pos].rfind(')').unwrap_or(pos)
    } else {
        return trimmed.to_string();
    };
    // 没有显式列名（如 INSERT INTO t VALUES (1)），无需添加 ON DUPLICATE KEY UPDATE
    if columns_start >= insert_head_end {
        return trimmed.to_string();
    }
    let columns_section = &trimmed[columns_start..insert_head_end];
    let columns = split_sql_columns(columns_section.trim_matches(|ch| ch == '(' || ch == ')'));
    if columns.is_empty() {
        return trimmed.to_string();
    }

    let assignments = columns
        .iter()
        .map(|column| format!("{0} = VALUES({0})", column))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "{} ON DUPLICATE KEY UPDATE {}",
        trimmed, assignments
    )
}

/// 在 SQL 大写文本中查找关键字，确保匹配的是独立关键字而非表名/列名的一部分。
/// 关键字前可以是空白、`)`、`` ` `` 或开头；后可以是空白、`(` 或结尾。
pub fn find_sql_keyword(upper: &str, keyword: &str) -> Option<usize> {
    let mut start = 0;
    let kw_len = keyword.len();
    while let Some(pos) = upper[start..].find(keyword) {
        let abs_pos = start + pos;
    let before_ok = abs_pos == 0 || {
        let c = upper.as_bytes()[abs_pos - 1];
        c.is_ascii_whitespace() || c == b')' || c == b'`' || c == b'('
    };
        let after_idx = abs_pos + kw_len;
        let after_ok = after_idx >= upper.len() || {
            let c = upper.as_bytes()[after_idx];
            c.is_ascii_whitespace() || c == b'('
        };
        if before_ok && after_ok {
            return Some(abs_pos);
        }
        start = abs_pos + 1;
    }
    None
}

fn split_sql_columns(columns: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut previous = '\0';

    for ch in columns.chars() {
        if ch == '`' && previous != '\\' {
            in_quote = !in_quote;
        }

        if ch == ',' && !in_quote {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                result.push(trimmed.to_string());
            }
            current.clear();
        } else {
            current.push(ch);
        }
        previous = ch;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        result.push(trimmed.to_string());
    }

    result
}

fn ensure_mysql_target_db(sql: &str, target_db: &str) -> String {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // 支持的 DML 语句前缀及其长度
    let dml_prefixes: &[(&str, usize)] = &[
        ("INSERT INTO ", "INSERT INTO ".len()),
        ("TRUNCATE TABLE ", "TRUNCATE TABLE ".len()),
        ("DELETE FROM ", "DELETE FROM ".len()),
        ("UPDATE ", "UPDATE ".len()),
    ];

    let (keyword, prefix_len) = match dml_prefixes
        .iter()
        .find(|(kw, _)| upper.starts_with(kw))
    {
        Some((kw, len)) => (kw.to_string(), *len),
        None => return trimmed.to_string(),
    };

    let after_prefix = &trimmed[prefix_len..];
    let mut chars = after_prefix.char_indices().peekable();
    let Some((_, first_char)) = chars.peek().copied() else {
        return trimmed.to_string();
    };

    if first_char != '`' {
        return trimmed.to_string();
    }

    let mut first_end = None;
    for (idx, ch) in after_prefix.char_indices().skip(1) {
        if ch == '`' {
            first_end = Some(idx);
            break;
        }
    }
    let Some(first_end) = first_end else {
        return trimmed.to_string();
    };

    let remaining = &after_prefix[first_end + 1..];
    if remaining.trim_start().starts_with('.') {
        return trimmed.to_string();
    }

    format!(
        "{}`{}`.{}",
        keyword,
        target_db.to_lowercase(),
        after_prefix
    )
}

pub fn read_csv_data(file_path: &str) -> Result<CsvData, String> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(file_path)
        .map_err(|e| format!("读取 CSV 文件失败: {}", e))?;

    let headers = reader
        .headers()
        .map_err(|e| format!("读取 CSV 表头失败: {}", e))?
        .clone();

    let columns: Vec<String> = headers.iter().map(|h| h.to_string()).collect();

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| format!("读取 CSV 行失败: {}", e))?;
        let row: Vec<DbValue> = record
            .iter()
            .map(|field| {
                if field.is_empty() {
                    DbValue::Null
                } else {
                    DbValue::Text(field.to_string())
                }
            })
            .collect();
        rows.push(row);
    }

    Ok(CsvData { columns, rows })
}

pub fn read_sql_data(file_path: &str) -> Result<String, String> {
    std::fs::read_to_string(file_path).map_err(|e| format!("读取 SQL 文件失败: {}", e))
}

/// 更新导入进度
fn update_progress(
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    task_id: &str,
    status: ImportStatus,
    progress: f64,
    total_rows: usize,
    imported_rows: usize,
    error_message: Option<String>,
) {
    if let Ok(mut map) = progress_map.lock() {
        map.insert(
            task_id.to_string(),
            ImportProgress {
                task_id: task_id.to_string(),
                status,
                progress,
                total_rows,
                imported_rows,
                error_message,
                errors: vec![],
            },
        );
    }
}

/// 初始化数据库表结构（从 Oracle DDL 转换并执行）
#[tauri::command]
pub fn list_schema_targets(schema_dir: String) -> Result<Vec<SchemaTarget>, String> {
    let converter = DdlConverter::new(PathBuf::from(schema_dir));
    converter.list_schema_targets()
}

/// 初始化数据库表结构（从 Oracle DDL 转换并执行）
#[tauri::command]
pub async fn init_schema(
    state: State<'_, AppState>,
    db_config_id: String,
    target_db: String,
    schema_dir: String,
) -> Result<String, String> {
    let db_config = {
        let configs = state.db_configs.lock().map_err(|e| e.to_string())?;
        configs
            .iter()
            .find(|c| c.id == db_config_id)
            .cloned()
            .ok_or_else(|| "未找到数据库配置".to_string())?
    };

    let converter = DdlConverter::new(PathBuf::from(schema_dir));
    converter.execute_ddl(&db_config, &target_db).await
}

/// 获取单张表的字段和注释信息
#[tauri::command]
pub fn get_table_schema(
    schema_dir: String,
    target_db: String,
    table_name: String,
) -> Result<TableSchemaInfo, String> {
    let converter = DdlConverter::new(PathBuf::from(schema_dir));
    converter.get_table_schema(&target_db, &table_name)
}

/// 一键初始化所有 Schema 目录下的数据库表结构
#[tauri::command]
pub async fn init_all_schemas(
    state: State<'_, AppState>,
    db_config_id: String,
    schema_dir: String,
) -> Result<String, String> {
    let db_config = {
        let configs = state.db_configs.lock().map_err(|e| e.to_string())?;
        configs
            .iter()
            .find(|c| c.id == db_config_id)
            .cloned()
            .ok_or_else(|| "未找到数据库配置".to_string())?
    };

    let converter = DdlConverter::new(PathBuf::from(&schema_dir));
    let targets = converter.list_schema_targets()?;

    let mut results = Vec::new();
    results.push(format!(
        "开始初始化 {} 个库的表结构...",
        targets.len()
    ));

    for target in &targets {
        results.push(format!("-- 初始化库: {} --", target.target_db));
        match converter.execute_ddl(&db_config, &target.target_db).await {
            Ok(log) => {
                results.push(log);
            }
            Err(e) => {
                results.push(format!("✗ 库 {} 初始化失败: {}", target.target_db, e));
            }
        }
    }

    results.push("所有库初始化完成".to_string());
    Ok(results.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{
        find_sql_keyword, import_target_table, prepare_sql_statements, read_csv_data,
        read_sql_data,
    };
    use crate::db::DbValue;
    use crate::models::{CsvFileInfo, DbType, ImportFileType};
    use std::fs;

    #[test]
    fn reads_empty_csv_fields_as_null_values() {
        let file = std::env::temp_dir().join(format!(
            "base-import-tool-null-csv-{}.csv",
            std::process::id()
        ));
        fs::write(&file, "id,kc_order,name\n1,,分类\n").unwrap();

        let data = read_csv_data(file.to_str().unwrap()).unwrap();

        assert_eq!(data.columns, vec!["id", "kc_order", "name"]);
        assert_eq!(
            data.rows[0],
            vec![
                DbValue::Text("1".to_string()),
                DbValue::Null,
                DbValue::Text("分类".to_string())
            ]
        );

        let _ = fs::remove_file(file);
    }

    #[test]
    fn builds_import_target_table_from_csv_database_folder() {
        let csv_file = CsvFileInfo {
            file_name: "attribute_dict.csv".to_string(),
            file_path: "/data/drug_spec/attribute_dict.csv".to_string(),
            file_type: ImportFileType::Csv,
            target_db: "drug_spec".to_string(),
            table_name: "attribute_dict".to_string(),
            row_count: None,
            columns: vec!["id".to_string()],
        };

        let table = import_target_table(&csv_file);

        assert_eq!(table.schema, "drug_spec");
        assert_eq!(table.table_name, "attribute_dict");
    }

    #[test]
    fn reads_sql_file_content_for_import() {
        let file = std::env::temp_dir().join(format!(
            "base-import-tool-import-sql-{}.sql",
            std::process::id()
        ));
        fs::write(&file, "insert into cbs.sys_user(id) values (1);").unwrap();

        let sql = read_sql_data(file.to_str().unwrap()).unwrap();

        assert_eq!(sql, "insert into cbs.sys_user(id) values (1);");

        let _ = fs::remove_file(file);
    }

    #[test]
    fn prepares_mysql_sql_statements_from_oracle_style_script() {
        let script = r#"
INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME", "ENGLISH_NAME") VALUES ('门诊', 'OUTPATIENT');
INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME") VALUES ('急诊');
"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0],
            "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`, `english_name`) VALUES ('门诊', 'OUTPATIENT') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`), `english_name` = VALUES(`english_name`)"
        );
        assert_eq!(
            statements[1],
            "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`) VALUES ('急诊') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`)"
        );
    }

    #[test]
    fn prepares_postgres_sql_statements_from_oracle_style_script() {
        let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME") VALUES ('门诊');"#;

        let statements = prepare_sql_statements(script, &DbType::PostgreSQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "INSERT INTO \"cbs\".\"dict_adm_route\" (\"admin_name\") VALUES ('门诊')"
        );
    }

    #[test]
    fn prepares_mysql_insert_select_with_duplicate_key_update() {
        let script =
            r#"INSERT INTO "CBS"."DICT_DOCTOR_TITLE" ("ID", "NAME") SELECT '1', '主任医师' FROM DUAL;"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "INSERT INTO `cbs`.`dict_doctor_title` (`id`, `name`) SELECT '1', '主任医师' FROM DUAL ON DUPLICATE KEY UPDATE `id` = VALUES(`id`), `name` = VALUES(`name`)"
        );
    }

    #[test]
    fn prepares_mysql_insert_without_schema_using_target_db() {
        let script = r#"INSERT INTO "DICT_DRUG_CATE" ("ID", "NAME") VALUES ('1', '西药');"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "INSERT INTO `cbs`.`dict_drug_cate` (`id`, `name`) VALUES ('1', '西药') ON DUPLICATE KEY UPDATE `id` = VALUES(`id`), `name` = VALUES(`name`)"
        );
    }

    #[test]
    fn prepares_mysql_truncate_without_schema_using_target_db() {
        let script = r#"TRUNCATE TABLE "DICT_DRUG_CATE";"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "TRUNCATE TABLE `cbs`.`dict_drug_cate`");
    }

    #[test]
    fn prepares_mysql_truncate_with_schema_keeps_prefix() {
        let script = r#"TRUNCATE TABLE "CBS"."DICT_DRUG_CATE";"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "TRUNCATE TABLE `cbs`.`dict_drug_cate`");
    }

    #[test]
    fn prepares_mysql_delete_without_schema_using_target_db() {
        let script = r#"DELETE FROM "DICT_DRUG_CATE";"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "DELETE FROM `cbs`.`dict_drug_cate`");
    }

    #[test]
    fn prepares_mysql_delete_with_schema_keeps_prefix() {
        let script = r#"DELETE FROM "CBS"."DICT_DRUG_CATE" WHERE ID = '1';"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "DELETE FROM `cbs`.`dict_drug_cate` WHERE ID = '1'"
        );
    }

    #[test]
    fn prepares_mysql_update_without_schema_using_target_db() {
        let script = r#"UPDATE "DICT_DRUG_CATE" SET NAME = 'test';"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "UPDATE `cbs`.`dict_drug_cate` SET NAME = 'test'"
        );
    }

    #[test]
    fn preserves_non_dml_statements() {
        let script = r#"SELECT 1 FROM DUAL;"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "SELECT 1 FROM DUAL");
    }

    #[test]
    fn handles_compact_values_format() {
        // 紧凑格式：)VALUES(，VALUES 前无空格
        let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME")VALUES('膀胱冲洗用');"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`)VALUES('膀胱冲洗用') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`)"
        );
    }

    #[test]
    fn handles_compact_values_format_with_trailing_space() {
        // 紧凑格式：)VALUES (，VALUES 后紧跟空格和左括号
        let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME")VALUES ('膀胱冲洗用');"#;

        let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs");

        assert_eq!(statements.len(), 1);
        assert!(
            statements[0].contains("ON DUPLICATE KEY UPDATE"),
            "should add ON DUPLICATE KEY UPDATE for compact VALUES format"
        );
    }

    #[test]
    fn find_sql_keyword_finds_with_space_before() {
        let upper = "INSERT INTO TBL (A) VALUES (1)";
        assert_eq!(find_sql_keyword(upper, "VALUES"), Some(20));
    }

    #[test]
    fn find_sql_keyword_finds_with_paren_before() {
        let upper = "INSERT INTO TBL (A)VALUES (1)";
        assert_eq!(find_sql_keyword(upper, "VALUES"), Some(19));
    }

    #[test]
    fn find_sql_keyword_finds_with_backtick_before() {
        let upper = "INSERT INTO `TBL` (`A`)VALUES (1)";
        assert_eq!(find_sql_keyword(upper, "VALUES"), Some(23));
    }

    #[test]
    fn find_sql_keyword_ignores_values_in_identifier() {
        let upper = "INSERT INTO MY_VALUES_TABLE (A) VALUES (1)";
        let pos = find_sql_keyword(upper, "VALUES").unwrap();
        // 应该匹配第二个 VALUES (关键字)，而不是 MY_VALUES_TABLE 中的 VALUES
        assert!(pos > 20, "should match keyword VALUES, not identifier prefix");
        assert_eq!(pos, 32);
    }
}
