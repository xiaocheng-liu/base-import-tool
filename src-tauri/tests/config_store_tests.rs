use base_import_tool_lib::config_store::{load_db_configs, save_db_configs};
use base_import_tool_lib::models::{DbConfig, DbType, TargetDb};
use std::fs;

#[test]
fn loads_configs_after_save() {
    let dir = std::env::temp_dir().join(format!(
        "base-import-tool-config-store-{}",
        std::process::id()
    ));
    let config_path = dir.join("db_configs.json");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let configs = vec![DbConfig {
        id: "config-1".to_string(),
        db_type: DbType::Oracle,
        target_db: TargetDb::Kbe,
        host: "127.0.0.1".to_string(),
        port: 1521,
        username: "kbe".to_string(),
        password: "secret".to_string(),
        database: "orclpdb".to_string(),
        extra_params: String::new(),
    }];

    save_db_configs(&config_path, &configs).unwrap();
    let loaded = load_db_configs(&config_path).unwrap();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "config-1");
    assert_eq!(loaded[0].host, "127.0.0.1");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loads_config_without_legacy_target_db() {
    let dir = std::env::temp_dir().join(format!(
        "base-import-tool-config-store-legacy-{}",
        std::process::id()
    ));
    let config_path = dir.join("db_configs.json");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        &config_path,
        r#"[{
          "id": "config-2",
          "db_type": "MySQL",
          "host": "localhost",
          "port": 3306,
          "username": "root",
          "password": "secret",
          "database": "mysql",
          "extra_params": ""
        }]"#,
    )
    .unwrap();

    let loaded = load_db_configs(&config_path).unwrap();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "config-2");
    assert_eq!(loaded[0].target_db, TargetDb::Kbe);

    let _ = fs::remove_dir_all(&dir);
}
