use crate::config_store;
use crate::csv_parser;
use crate::db::{self, DbConnection, DbValue};
use crate::ddl_converter::DdlConverter;
use crate::models::*;
use csv::ReaderBuilder;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};
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

/// 获取表字段和注释信息（从 DDL 文件解析）
#[tauri::command]
pub fn get_table_schema(
    schema_dir: String,
    target_db: String,
    table_name: String,
) -> Result<TableSchemaInfo, String> {
    let converter = DdlConverter::new(PathBuf::from(schema_dir));
    converter.get_table_schema(&target_db, &table_name)
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
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    csv_files: Vec<CsvFileInfo>,
    db_config_id: String,
    truncate_first: bool,
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
    let _ = app_handle.emit("import-log", format!("▶ 连接数据库成功 ({})", db_config.db_type));

    // 先清空所有目标表
    if truncate_first {
        let _ = app_handle.emit("import-log", "▶ 开始清空目标表...");
        for csv_file in &csv_files {
            let target_table = import_target_table(csv_file);
            let table_label = format!("{}.{}", target_table.schema, target_table.table_name);
            match conn.schema_table_exists(&target_table).await {
                Ok(true) => {
                    // 优先尝试 TRUNCATE（快），失败则降级为 DELETE（兼容外键约束）
                    let truncate_sql = match &db_config.db_type {
                        DbType::MySQL => format!("TRUNCATE TABLE `{}`.`{}`", target_table.schema.to_lowercase(), target_table.table_name.to_lowercase()),
                        DbType::PostgreSQL => format!("TRUNCATE TABLE \"{}\".\"{}\"", target_table.schema.to_lowercase(), target_table.table_name.to_lowercase()),
                        _ => format!("TRUNCATE TABLE \"{}\".\"{}\"", target_table.schema.to_uppercase(), target_table.table_name.to_uppercase()),
                    };
                    let _ = app_handle.emit("import-log", format!("  SQL: {}", truncate_sql));
                    match conn.execute_raw_sql(&truncate_sql).await {
                        Ok(_) => {
                            let _ = app_handle.emit("import-log", format!("  ✓ 清空表 {}", table_label));
                        }
                        Err(truncate_err) => {
                            let _ = app_handle.emit("import-log", format!("  ⚠ TRUNCATE 失败: {}，尝试 DELETE...", truncate_err));
                            let delete_sql = match &db_config.db_type {
                                DbType::MySQL => format!("DELETE FROM `{}`.`{}`", target_table.schema.to_lowercase(), target_table.table_name.to_lowercase()),
                                DbType::PostgreSQL => format!("DELETE FROM \"{}\".\"{}\"", target_table.schema.to_lowercase(), target_table.table_name.to_lowercase()),
                                _ => format!("DELETE FROM \"{}\".\"{}\"", target_table.schema.to_uppercase(), target_table.table_name.to_uppercase()),
                            };
                            let _ = app_handle.emit("import-log", format!("  SQL: {}", delete_sql));
                            conn.execute_raw_sql(&delete_sql).await
                                .map_err(|e| format!("清空表 {} 失败: {}", table_label, e))?;
                            let _ = app_handle.emit("import-log", format!("  ✓ 清空表 {}", table_label));
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => return Err(format!("检查表 {} 是否存在失败: {}", table_label, e)),
            }
        }
    }

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
                },
            );
        }

        tasks.push(task);
    }

    // 异步并发执行导入：每个表独立连接、独立 tokio 任务
    let progress_map = Arc::clone(&state.import_progress);
    let db_config_for_spawn = db_config.clone();
    let total_tasks = tasks.len();
    let tasks_for_spawn = tasks.clone();
    tokio::spawn(async move {
        // 限制最大并发数，避免过多连接压垮数据库
        let max_concurrent = 5usize;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut join_set = tokio::task::JoinSet::new();

        for task in &tasks_for_spawn {
            let permit = Arc::clone(&semaphore);
            let task = task.clone();
            let progress_map = Arc::clone(&progress_map);
            let db_config = db_config_for_spawn.clone();
            let db_type = db_type.clone();
            let app_handle = app_handle.clone();

            join_set.spawn(async move {
                let _permit = permit.acquire().await;
                let _ = app_handle.emit(
                    "import-log",
                    format!("▶ 开始导入 [{}.{}]", task.csv_file.target_db, task.csv_file.table_name),
                );
                match db::create_connection(&db_config).await {
                    Ok(conn) => {
                        execute_import(&conn, &task, &progress_map, &db_type, truncate_first, &app_handle).await;
                        // conn 在离开作用域时自动释放
                    }
                    Err(e) => {
                        update_progress(
                            &progress_map,
                            &task.id,
                            ImportStatus::Failed,
                            0.0,
                            0,
                            0,
                            Some(format!("创建数据库连接失败: {}", e)),
                        );
                    }
                }
            });
        }

        // 等待所有导入任务完成
        while join_set.join_next().await.is_some() {}

        let _ = app_handle.emit(
            "import-log",
            format!("══════ 导入完成 ({} 个表) ══════", total_tasks),
        );
    });

    Ok(tasks)
}

/// 执行单个文件的导入
async fn execute_import(
    conn: &Box<dyn DbConnection>,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    db_type: &DbType,
    truncate_first: bool,
    app_handle: &tauri::AppHandle,
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
        ImportFileType::Csv => execute_csv_import(conn, task, progress_map, app_handle).await,
        ImportFileType::Sql => execute_sql_import(conn, task, progress_map, db_type, truncate_first, app_handle).await,
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
    conn: &Box<dyn DbConnection>,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    app_handle: &tauri::AppHandle,
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
    let _ = app_handle.emit("import-log", format!("  CSV 导入: {} 行", total_rows));

    // 更新 total_rows 为实际读取的行数
    {
        if let Ok(mut map) = progress_map.lock() {
            if let Some(p) = map.get_mut(&task.id) {
                p.total_rows = total_rows;
            }
        }
    }

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
    conn: &Box<dyn DbConnection>,
    task: &ImportTask,
    progress_map: &Arc<Mutex<HashMap<String, ImportProgress>>>,
    db_type: &DbType,
    truncate_first: bool,
    app_handle: &tauri::AppHandle,
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

    let statements = prepare_sql_statements(&sql, db_type, &task.csv_file.target_db, truncate_first);
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
    let _ = app_handle.emit("import-log", format!("  ▶ 共 {} 条 SQL 语句", total));
    for (index, statement) in statements.iter().enumerate() {
        let _ = app_handle.emit("import-log", format!("  SQL ({}/{}): {}", index + 1, total, statement));
        if let Err(e) = conn.execute_raw_sql(statement).await {
            let error_msg = format_error_message(&e, statement);
            update_progress(
                progress_map,
                &task.id,
                ImportStatus::Failed,
                0.0,
                total,
                index,
                Some(error_msg),
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

/// 格式化错误信息，提供更友好的错误提示和解决建议。
fn format_error_message(e: &str, statement: &str) -> String {
    let mut msg = format!("执行 SQL 文件失败: {}", e);
    
    // 打印完整的 SQL 语句（不截断）
    msg.push_str(&format!("\n\n出错 SQL (完整):\n{}", statement));
    
    // 根据错误类型提供解决建议
    if e.contains("1264") || e.contains("Out of range") {
        msg.push_str("\n\n解决建议：");
        msg.push_str("\n1. 检查 SQL 文件中对应列的值是否超出目标表列的范围");
        msg.push_str("\n2. 检查目标表列的类型是否为 TINYINT（范围 -128~127 或 0~255）");
        msg.push_str("\n3. 如果是，请将目标表列类型改为 INT 或 BIGINT");
        msg.push_str("\n4. 或者修改 SQL 文件，将超出范围的值改为在范围内的");
    } else if e.contains("1411") || e.contains("Incorrect datetime value") {
        msg.push_str("\n\n解决建议：");
        msg.push_str("\n1. 检查 SQL 文件中日期值的格式是否与 TO_DATE 的格式字符串匹配");
        msg.push_str("\n2. 如果日期值是标准格式（YYYY-MM-DD HH:MM:SS），可以尝试去掉 TO_DATE 函数");
        msg.push_str("\n3. 或者修改 TO_DATE 的格式字符串，使其与日期值匹配");
    } else if e.contains("1062") || e.contains("Duplicate entry") {
        msg.push_str("\n\n解决建议：");
        msg.push_str("\n1. 检查是否有重复的数据");
        msg.push_str("\n2. 如果勾选了'清空表'，请确保清空操作成功执行");
        msg.push_str("\n3. 或者手动删除目标表中的数据后再导入");
    }
    
    msg
}

pub fn prepare_sql_statements(sql: &str, db_type: &DbType, target_db: &str, truncate_first: bool) -> Vec<String> {
    split_sql_statements(sql)
        .into_iter()
        .map(|stmt| adapt_sql_statement(&stmt, db_type, target_db, truncate_first))
        .map(|stmt| {
            // 去除每个语句末尾的注释行
            let cleaned = strip_trailing_line_comments(&stmt);
            if cleaned.trim().is_empty() {
                String::new()
            } else {
                cleaned
            }
        })
        .filter(|stmt| {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                return false;
            }
            // 过滤掉 COMMIT / ROLLBACK 等事务控制语句
            let upper = trimmed.to_uppercase();
            if upper == "COMMIT" || upper == "ROLLBACK" || upper.starts_with("COMMIT;") || upper.starts_with("ROLLBACK;") {
                return false;
            }
            // 过滤掉纯注释或仅含注释+空白的分号块
            !is_comment_only(trimmed)
        })
        .collect()
}

/// 去除 SQL 语句末尾的注释行（-- 开头的行注释），返回去除注释后的 SQL。
fn strip_trailing_line_comments(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let mut end = lines.len();
    
    // 从末尾向前找，找到第一个非注释非空行
    for (i, line) in lines.iter().enumerate().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            end = i;
        } else {
            break;
        }
    }
    
    if end == 0 {
        return String::new();
    }
    
    lines[..end].join("\n")
}

/// 判断字符串是否仅由 SQL 注释和空白组成（无实际 SQL 语句）。
fn is_comment_only(s: &str) -> bool {
    // 先去掉末尾的注释行
    let cleaned = strip_trailing_line_comments(s);
    if cleaned.is_empty() {
        return true;
    }
    
    let mut in_block_comment = false;
    for line in cleaned.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        // 处理块注释跨行的情况
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        
        // 检查是否有 /* 开头的块注释
        if trimmed.starts_with("/*") {
            if !trimmed.ends_with("*/") {
                in_block_comment = true;
            }
            continue;
        }
        
        if trimmed.starts_with("--") {
            continue;
        }
        
        // 检查是否是 SET 语句（如 SET DEFINE OFF）——这些也应该被过滤
        let upper = trimmed.to_uppercase();
        if upper.starts_with("SET ") || upper.starts_with("PROMPT ") {
            continue;
        }
        
        // 有非注释内容
        return false;
    }
    // 如果还在块注释中，也算注释
    true
}

/// 去除 SQL 开头的注释行（-- 和 /* */ 块注释），返回去除注释后的 SQL。
fn strip_leading_comments(sql: &str) -> &str {
    let bytes = sql.as_bytes();
    let mut pos = 0;
    let len = bytes.len();
    
    while pos < len {
        // 跳过空白
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        
        if pos >= len {
            break;
        }
        
        // 检查 -- 行注释
        if pos + 1 < len && bytes[pos] == b'-' && bytes[pos + 1] == b'-' {
            // 跳到行尾
            while pos < len && bytes[pos] != b'\n' && bytes[pos] != b'\r' {
                pos += 1;
            }
            continue;
        }
        
        // 检查 /* 块注释
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            // 找到 */
            pos += 2;
            while pos + 1 < len {
                if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                    pos += 2;
                    break;
                }
                pos += 1;
            }
            continue;
        }
        
        // 不是注释，停止
        break;
    }
    
    &sql[pos..]
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    // 先去除开头的注释
    let sql = strip_leading_comments(sql);
    
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut previous = '\0';

    for ch in sql.chars() {
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

fn adapt_sql_statement(sql: &str, db_type: &DbType, target_db: &str, truncate_first: bool) -> String {
    let sql = convert_oracle_functions(sql, db_type);
    match db_type {
        DbType::MySQL => {
            let sql = ensure_mysql_target_db(
                &replace_quoted_identifiers(&sql, '`', true),
                target_db,
            );
            if truncate_first {
                sql
            } else {
                with_mysql_duplicate_key_update(&sql)
            }
        },
        DbType::PostgreSQL => replace_quoted_identifiers(&sql, '"', true),
        _ => sql.trim().to_string(),
    }
}

/// 将 Oracle 特有函数转换为目标数据库的等价函数。
fn convert_oracle_functions(sql: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::MySQL => convert_to_date_for_mysql(sql),
        DbType::PostgreSQL => convert_to_date_for_postgres(sql),
        _ => sql.to_string(),
    }
}

/// 将 Oracle TO_DATE(d, fmt) 转换为 MySQL STR_TO_DATE(d, fmt)。
fn convert_to_date_for_mysql(sql: &str) -> String {
    // 打印转换前的 SQL（用于调试）
    // println!("Before conversion: {}", sql);
    let result = replace_oracle_function(sql, "TO_DATE", "STR_TO_DATE", |fmt| {
        let converted = oracle_date_fmt_to_mysql(fmt);
        // println!("Format conversion: {} -> {}", fmt, converted);
        clean_mysql_date_fmt(&converted)
    });
    // println!("After conversion: {}", result);
    result
}

/// 将 Oracle TO_DATE(d, fmt) 转换为 PostgreSQL TO_TIMESTAMP(d, fmt)。
fn convert_to_date_for_postgres(sql: &str) -> String {
    replace_oracle_function(sql, "TO_DATE", "TO_TIMESTAMP", |fmt| {
        oracle_date_fmt_to_postgres(fmt)
    })
}

/// 通用 Oracle 函数替换：将 func_name(args) 替换为 target_func(transformed_args)。
/// transform_fn 对第一个参数后的格式字符串进行转换。
fn replace_oracle_function<F>(sql: &str, func_name: &str, target_func: &str, transform_fn: F) -> String
where
    F: Fn(&str) -> String,
{
    let upper_sql = sql.to_uppercase();
    let upper_func = func_name.to_uppercase();
    let mut result = String::with_capacity(sql.len());
    let mut search_start = 0usize;

    while let Some(pos) = upper_sql[search_start..].find(&upper_func) {
        let abs_pos = search_start + pos;

        // 检查前面是否是合法边界（空白、逗号、(、行首）
        let before_ok = abs_pos == 0 || {
            let c = sql.as_bytes()[abs_pos - 1];
            c.is_ascii_whitespace() || c == b',' || c == b'(' || c == b'=' || c == b'>' || c == b'<'
        };

        // 检查后面是否是 (
        let after_func = abs_pos + upper_func.len();
        let after_ok = after_func < sql.len() && sql.as_bytes()[after_func] == b'(';

        if !before_ok || !after_ok {
            // 不是函数调用，继续搜索
            result.push_str(&sql[search_start..after_func]);
            search_start = after_func;
            continue;
        }

        // 找到函数调用，提取参数
        result.push_str(&sql[search_start..abs_pos]);
        result.push_str(target_func);

        // 解析括号内的参数
        let (args_str, paren_end) = extract_parenthesized_args(&sql, after_func);

        // 转换参数：只转换第二个参数（格式字符串）
        let converted_args = transform_oracle_func_args(&args_str, &transform_fn);
        result.push('(');
        result.push_str(&converted_args);
        result.push(')');

        search_start = paren_end + 1;
    }

    result.push_str(&sql[search_start..]);
    result
}

/// 提取括号内的参数内容，返回 (参数内容, 右括号位置)。
fn extract_parenthesized_args(sql: &str, open_paren_pos: usize) -> (String, usize) {
    let bytes = sql.as_bytes();
    let mut depth = 0u32;
    let mut in_single_quote = false;
    let start = open_paren_pos + 1; // 跳过 (

    for (i, &ch) in bytes.iter().enumerate().skip(start) {
        if ch == b'\'' {
            in_single_quote = !in_single_quote;
            continue;
        }
        if in_single_quote {
            continue;
        }
        if ch == b'(' {
            depth += 1;
        } else if ch == b')' {
            if depth == 0 {
                return (sql[start..i].to_string(), i);
            }
            depth -= 1;
        }
    }

    // 未找到匹配的右括号，返回从 ( 到末尾的内容
    (sql[start..].to_string(), sql.len() - 1)
}

/// 转换 Oracle 函数参数列表：第二个参数（格式字符串）应用 transform_fn。
fn transform_oracle_func_args<F>(args: &str, transform_fn: &F) -> String
where
    F: Fn(&str) -> String,
{
    let parts = split_args_preserving_quotes(args);
    if parts.len() < 2 {
        return args.to_string();
    }

    let mut result = parts[0].clone();
    for (i, part) in parts.iter().enumerate().skip(1) {
        result.push(',');
        if i == 1 {
            // 第二个参数是格式字符串，去掉外层引号后转换，再包裹回去
            let trimmed = part.trim();
            let inner = trim_outer_quotes(trimmed);
            let transformed = transform_fn(&inner);
            let leading_spaces: String = part.chars().take_while(|c| c.is_whitespace()).collect();
            result.push_str(&leading_spaces);
            result.push('\'');
            result.push_str(&transformed);
            result.push('\'');
        } else {
            result.push_str(part);
        }
    }
    result
}

/// 去掉字符串外层匹配的单引号或双引号。
fn trim_outer_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// 按逗号分割参数，保留引号内逗号。
fn split_args_preserving_quotes(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let bytes = s.as_bytes();

    for &ch in bytes {
        if ch == b'\'' {
            in_single_quote = !in_single_quote;
            current.push(ch as char);
        } else if ch == b',' && !in_single_quote {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(ch as char);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Oracle 日期格式模型 → MySQL STR_TO_DATE 格式模型。
fn oracle_date_fmt_to_mysql(fmt: &str) -> String {
    let mut result = String::with_capacity(fmt.len());
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' {
            // 字面量文本，原样保留
            result.push('\'');
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                result.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                result.push('\'');
                i += 1;
            }
        } else if i + 1 < chars.len() && chars[i] == 'M' && chars[i + 1] == 'I' {
            // MI → %i (minutes)
            result.push_str("%i");
            i += 2;
        } else if i + 1 < chars.len() && chars[i] == 'S' && chars[i + 1] == 'S' {
            // SS → %s (seconds)
            result.push_str("%s");
            i += 2;
        } else if i + 1 < chars.len() && (chars[i] == 'A' || chars[i] == 'P') && chars[i + 1] == 'M' {
            // AM/PM → %p
            result.push_str("%p");
            i += 2;
        } else if chars[i] == 'Y' || chars[i] == 'y' {
            let start = i;
            while i < chars.len() && (chars[i] == 'Y' || chars[i] == 'y') {
                i += 1;
            }
            let count = i - start;
            match count {
                4 => result.push_str("%Y"),
                2 => result.push_str("%y"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'M' {
            let start = i;
            while i < chars.len() && chars[i] == 'M' {
                i += 1;
            }
            let count = i - start;
            match count {
                1 => result.push_str("%c"),
                2 => result.push_str("%m"),
                3 | 4 => result.push_str("%M"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'D' {
            let start = i;
            while i < chars.len() && chars[i] == 'D' {
                i += 1;
            }
            let count = i - start;
            match count {
                1 => result.push_str("%e"),
                2 => result.push_str("%d"),
                3 | 4 => result.push_str("%W"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'H' {
            let start = i;
            while i < chars.len() && chars[i] == 'H' {
                i += 1;
            }
            let count = i - start;
            match count {
                2 if start + 2 < chars.len() && chars[start + 2] == '2' && chars[start + 3] == '4' => {
                    // HH24 → %H
                    result.push_str("%H");
                    i += 2;
                }
                2 => result.push_str("%H"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else {
            // 分隔符等，原样保留
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// 清理转换后的 MySQL 日期格式字符串，去除可能的多余字符。
/// 例如，如果格式字符串以非 % 开头的字母开头，可能是转换残留，需要清理。
fn clean_mysql_date_fmt(fmt: &str) -> String {
    // 如果格式字符串以字母开头且不是 %，则去掉开头的字母
    let mut chars = fmt.chars();
    if let Some(first) = chars.next() {
        if first.is_alphabetic() && first != '%' {
            // 去掉开头的所有字母，直到遇到 % 或非字母
            let mut result = String::new();
            let mut found_percent = false;
            for c in fmt.chars() {
                if c == '%' {
                    found_percent = true;
                }
                if found_percent {
                    result.push(c);
                }
            }
            return result;
        }
    }
    fmt.to_string()
}

/// Oracle 日期格式模型 → PostgreSQL TO_TIMESTAMP 格式模型。
fn oracle_date_fmt_to_postgres(fmt: &str) -> String {
    let mut result = String::with_capacity(fmt.len());
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' {
            result.push('"');
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                result.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                result.push('"');
                i += 1;
            }
        } else if i + 1 < chars.len() && chars[i] == 'M' && chars[i + 1] == 'I' {
            // MI → MI (PostgreSQL keeps same)
            result.push_str("MI");
            i += 2;
        } else if i + 1 < chars.len() && chars[i] == 'S' && chars[i + 1] == 'S' {
            // SS → SS
            result.push_str("SS");
            i += 2;
        } else if i + 1 < chars.len() && (chars[i] == 'A' || chars[i] == 'P') && chars[i + 1] == 'M' {
            result.push_str(&fmt[i..i + 2]);
            i += 2;
        } else if chars[i] == 'Y' || chars[i] == 'y' {
            let start = i;
            while i < chars.len() && (chars[i] == 'Y' || chars[i] == 'y') {
                i += 1;
            }
            let count = i - start;
            match count {
                4 => result.push_str("YYYY"),
                2 => result.push_str("YY"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'M' {
            let start = i;
            while i < chars.len() && chars[i] == 'M' {
                i += 1;
            }
            let count = i - start;
            match count {
                2 => result.push_str("MM"),
                3 | 4 => result.push_str("Month"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'D' {
            let start = i;
            while i < chars.len() && chars[i] == 'D' {
                i += 1;
            }
            let count = i - start;
            match count {
                2 => result.push_str("DD"),
                3 | 4 => result.push_str("Day"),
                _ => result.push_str(&fmt[start..i]),
            }
        } else if chars[i] == 'H' {
            let start = i;
            while i < chars.len() && chars[i] == 'H' {
                i += 1;
            }
            if start + 2 < chars.len() && chars.get(start + 2) == Some(&'2') && chars.get(start + 3) == Some(&'4') {
                result.push_str("HH24");
                i = start + 4;
            } else {
                result.push_str(&fmt[start..i]);
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
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
        // 需要确保 ) 在 ( 之后，否则说明 VALUES 在列括号之前（如 INSERT INTO t VALUES ...）
        upper[..pos].rfind(')').filter(|&r| r > columns_start).unwrap_or(pos)
    } else if let Some(pos) = upper.find(" SELECT ") {
        pos
    } else if let Some(pos) = find_sql_keyword(&upper, "SELECT") {
        upper[..pos].rfind(')').filter(|&r| r > columns_start).unwrap_or(pos)
    } else {
        return trimmed.to_string();
    };
    // 确保 columns_start 在 insert_head_end 之前
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
            c.is_ascii_whitespace() || c == b')' || c == b'`'
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
        ("CREATE TABLE IF NOT EXISTS ", "CREATE TABLE IF NOT EXISTS ".len()),
        ("CREATE TABLE ", "CREATE TABLE ".len()),
        ("ALTER TABLE ", "ALTER TABLE ".len()),
        ("DROP TABLE IF EXISTS ", "DROP TABLE IF EXISTS ".len()),
        ("DROP TABLE ", "DROP TABLE ".len()),
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

/// 一次性初始化所有库的表结构
#[tauri::command]
pub async fn init_all_schemas(
    app_handle: tauri::AppHandle,
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

    let converter = DdlConverter::new(PathBuf::from(schema_dir));
    let targets = converter.list_schema_targets()?;
    let mut results = Vec::new();

    for target in &targets {
        let _ = app_handle.emit("schema-log", format!("▶ 开始初始化 [{}]", target.target_db.to_uppercase()));
        match converter.execute_ddl(&db_config, &target.target_db).await {
            Ok(msg) => {
                let log_lines: Vec<&str> = msg.lines().collect();
                for line in log_lines {
                    let _ = app_handle.emit("schema-log", line.to_string());
                }
                results.push(format!("[{}] {}", target.target_db.to_uppercase(), msg));
            }
            Err(e) => {
                let _ = app_handle.emit("schema-log", format!("✗ [{}] 失败: {}", target.target_db.to_uppercase(), e));
                results.push(format!("[{}] 失败: {}", target.target_db.to_uppercase(), e));
            }
        }
    }

    let _ = app_handle.emit("schema-log", "══════ 初始化完成 ══════");

    Ok(results.join("\n\n"))
}

