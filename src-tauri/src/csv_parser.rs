use crate::models::{CsvFileInfo, ImportFileType, TargetDb};
use std::fs;
use std::path::Path;

/// 扫描文件夹，查找所有 CSV 文件并按目标数据库分组
pub fn scan_folder(folder_path: &str) -> Result<Vec<CsvFileInfo>, String> {
    let path = Path::new(folder_path);
    if !path.exists() || !path.is_dir() {
        return Err(format!("文件夹不存在或不是目录: {}", folder_path));
    }

    let mut files = Vec::new();
    collect_csv_files(path, &mut files)?;

    // 按 target_db 和 table_name 排序
    files.sort_by(|a, b| {
        a.target_db
            .cmp(&b.target_db)
            .then(a.table_name.cmp(&b.table_name))
    });

    Ok(files)
}

const KNOWN_TARGET_DBS: [&str; 9] = [
    "clin_wkst",
    "drug_spec",
    "kb_docs",
    "procure",
    "outpt",
    "inpt",
    "cbs",
    "kbe",
    "his",
];

/// 递归收集导入文件。
fn collect_csv_files(path: &Path, files: &mut Vec<CsvFileInfo>) -> Result<(), String> {
    let entries = fs::read_dir(path).map_err(|e| format!("读取文件夹失败: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
        let file_path = entry.path();

        if file_path.is_dir() {
            collect_csv_files(&file_path, files)?;
            continue;
        }

        // 处理 .csv / .sql 文件
        if file_path.is_file() {
            if let Some(ext) = file_path.extension() {
                if let Some(file_name) = file_path.file_name() {
                    let file_name = file_name.to_string_lossy().to_string();
                    match ext.to_string_lossy().to_ascii_lowercase().as_str() {
                        "csv" => {
                            if should_skip_csv_file(&file_name) {
                                continue;
                            }
                            if let Some(target_db) = infer_target_db(&file_path) {
                                let table_name = extract_table_name(&file_name, "csv");

                                // 扫描阶段只读取表头，避免大文件统计行数导致卡顿
                                let columns = parse_csv_headers(&file_path).unwrap_or_default();

                                files.push(CsvFileInfo {
                                    file_name: file_name.clone(),
                                    file_path: file_path.to_string_lossy().to_string(),
                                    file_type: ImportFileType::Csv,
                                    target_db: target_db.to_string(),
                                    table_name,
                                    row_count: None,
                                    columns,
                                });
                            } else {
                                log::warn!("无法识别文件所属数据库: {}", file_name);
                            }
                        }
                        "sql" => {
                            if let Some((target_db, table_name)) = infer_sql_target(&file_name) {
                                files.push(CsvFileInfo {
                                    file_name: file_name.clone(),
                                    file_path: file_path.to_string_lossy().to_string(),
                                    file_type: ImportFileType::Sql,
                                    target_db,
                                    table_name,
                                    row_count: None,
                                    columns: Vec::new(),
                                });
                            } else {
                                log::warn!("无法识别 SQL 文件所属数据库: {}", file_name);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

/// 跳过导出过程中的辅助文件，避免被误当成业务表导入。
fn should_skip_csv_file(file_name: &str) -> bool {
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "datamanage_progress.csv"
    )
}

/// 推断 CSV 所属目标数据库，只使用父目录名。
fn infer_target_db(file_path: &Path) -> Option<TargetDb> {
    file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .and_then(TargetDb::from_dir_name)
}

/// 从文件名提取表名。
fn extract_table_name(filename: &str, extension: &str) -> String {
    filename
        .strip_suffix(&format!(".{}", extension))
        .unwrap_or(filename)
        .to_string()
}

/// 从 SQL 文件名推断目标库和表名。
fn infer_sql_target(file_name: &str) -> Option<(String, String)> {
    let base_name = file_name.strip_suffix(".sql")?;
    for db_name in KNOWN_TARGET_DBS {
        let prefix = format!("{}_", db_name);
        if let Some(table_name) = base_name.strip_prefix(&prefix) {
            if !table_name.is_empty() {
                return Some((db_name.to_string(), table_name.to_string()));
            }
        }
    }
    None
}

/// 解析 CSV 文件表头。
fn parse_csv_headers(file_path: &Path) -> Result<Vec<String>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(file_path)
        .map_err(|e| format!("读取 CSV 文件失败: {}", e))?;

    // 获取表头（列名）
    let headers = reader
        .headers()
        .map_err(|e| format!("读取 CSV 表头失败: {}", e))?
        .clone();

    let columns: Vec<String> = headers.iter().map(|h| h.to_string()).collect();

    Ok(columns)
}

