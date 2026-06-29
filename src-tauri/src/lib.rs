pub mod commands;
pub mod config_store;
pub mod csv_parser;
pub mod db;
pub mod ddl_converter;
pub mod models;

use commands::AppState;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// 根据平台返回 Oracle 客户端资源目录名。
fn oracle_client_resource_dir() -> &'static str {
    if cfg!(target_os = "windows") {
        "oracle-client-windows"
    } else if cfg!(target_os = "linux") {
        "oracle-client-linux"
    } else {
        "oracle-client-macos"
    }
}

/// 配置随应用打包的 Oracle Instant Client（优先使用打包版本）。
pub fn configure_bundled_oracle_client(oracle_client_dir: &Path) {
    if has_oracle_library(oracle_client_dir) {
        env::set_var("ORACLE_CLIENT_LIB_DIR", oracle_client_dir.as_os_str());
    }
}

/// 检查目录中是否包含当前平台的 Oracle 客户端库文件。
pub fn has_oracle_library(dir: &Path) -> bool {
    let lib_name = if cfg!(target_os = "windows") {
        "oci.dll"
    } else if cfg!(target_os = "linux") {
        "libclntsh.so"
    } else {
        "libclntsh.dylib"
    };
    dir.join(lib_name).exists()
}

/// 解析可用的 Oracle Instant Client 目录。
pub fn resolve_oracle_client_dir(
    dev_oracle_client_dir: Option<&Path>,
    bundled_oracle_client_dir: Option<&Path>,
) -> Option<PathBuf> {
    dev_oracle_client_dir
        .filter(|dir| has_oracle_library(dir))
        .map(Path::to_path_buf)
        .or_else(|| {
            bundled_oracle_client_dir
                .filter(|dir| has_oracle_library(dir))
                .map(Path::to_path_buf)
        })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let resource_dir_name = oracle_client_resource_dir();
            let bundled_oracle_client_dir = app
                .path()
                .resolve(resource_dir_name, tauri::path::BaseDirectory::Resource)
                .ok();
            let dev_oracle_client_dir = app
                .path()
                .app_local_data_dir()
                .ok()
                .and_then(|_| std::env::current_dir().ok())
                .map(|current_dir| {
                    current_dir.join(format!("src-tauri/resources/{}", resource_dir_name))
                });

            if let Some(oracle_client_dir) = resolve_oracle_client_dir(
                dev_oracle_client_dir.as_deref(),
                bundled_oracle_client_dir.as_deref(),
            ) {
                configure_bundled_oracle_client(&oracle_client_dir);
            }

            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("获取配置目录失败: {}", e))?;
            let db_config_path = config_dir.join("db_configs.json");
            let db_configs = config_store::load_db_configs(&db_config_path)?;

            app.manage(AppState {
                db_configs: Mutex::new(db_configs),
                db_config_path,
                import_progress: Arc::new(Mutex::new(HashMap::new())),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_csv_files,
            commands::get_table_schema,
            commands::get_db_configs,
            commands::save_db_config,
            commands::delete_db_config,
            commands::test_connection,
            commands::start_import,
            commands::get_import_progress,
            commands::list_schema_targets,
            commands::init_schema,
            commands::init_all_schemas,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
