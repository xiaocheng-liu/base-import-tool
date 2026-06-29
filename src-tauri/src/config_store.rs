use crate::models::DbConfig;
use std::fs;
use std::path::Path;

/// 加载数据库连接配置。
pub fn load_db_configs(config_path: &Path) -> Result<Vec<DbConfig>, String> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let content =
        fs::read_to_string(config_path).map_err(|e| format!("读取数据库配置失败: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("解析数据库配置失败: {}", e))
}

/// 保存数据库连接配置。
pub fn save_db_configs(config_path: &Path, configs: &[DbConfig]) -> Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }

    let content = serde_json::to_string_pretty(configs)
        .map_err(|e| format!("序列化数据库配置失败: {}", e))?;
    fs::write(config_path, content).map_err(|e| format!("保存数据库配置失败: {}", e))
}

